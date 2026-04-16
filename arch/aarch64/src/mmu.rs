//! AArch64 MMU initialisation — identity mapping + SCTLR_EL1.M enable.
//!
//! Builds a static identity page-table (VA == PA) and enables the MMU so that
//! subsequent code can rely on cache coherency and normal memory semantics.
//!
//! ## Coverage
//!
//! **Default (QEMU virt):**
//!   Blocks 0–7 → 0 to 8 GiB, Normal WB/WA inner-shareable.
//!   QEMU virt has up to 8 GiB RAM starting at 0x4000_0000.
//!
//! **RPi 5 (`rpi5` feature):**
//!   Blocks 0–7  → 0 to 8 GiB, Normal WB/WA   (4/8 GiB LPDDR4X RAM)
//!   Block  65   → 65 to 66 GiB, Device nGnRnE  (RP1 MMIO + GIC-400)
//!
//!   RPi 5 MMIO addresses and the 1 GiB block they fall in:
//!     UART0 (RP1)  0x107D_0010_00  →  block 65  (65×1GiB – 66×1GiB)
//!     GICD         0x107F_FF90_00  →  block 65
//!     GICC         0x107F_FFA0_00  →  block 65
//!
//!   Block index = ⌊PA / 1 GiB⌋ = ⌊0x107D001000 / 0x4000_0000⌋ = 65.

/// Each page-table level is 512 × 8-byte entries = 4 KiB; must be 4-KiB aligned.
#[repr(C, align(4096))]
struct PageTable([u64; 512]);

// SAFETY: single-CPU, called once before any other code uses these tables.
static mut ID_L0: PageTable = PageTable([0u64; 512]);
static mut ID_L1: PageTable = PageTable([0u64; 512]);

/// Enable the AArch64 MMU with an identity mapping.
///
/// No-op if the MMU is already on (SCTLR_EL1.M == 1).
///
/// # Safety
/// Must be called from EL1 with MAIR_EL1 already programmed (index 0 =
/// Normal WB/WA, index 1 = Device nGnRnE).  No other CPU must be running
/// translation table walks concurrently.
pub unsafe fn enable_identity() {
    // Read SCTLR_EL1; skip if MMU already enabled.
    let sctlr: u64;
    core::arch::asm!("mrs {v}, SCTLR_EL1", v = out(reg) sctlr, options(nostack, nomem));
    if sctlr & 1 != 0 { return; }

    // ── L1 block descriptor attribute words ──────────────────────────────────
    //
    // Bits common to both:
    //   [1:0] = 0b01  → block descriptor
    //   [10]  = 1     → AF (Access Flag; avoids permission fault on first access)
    //
    // Normal WB/WA inner-shareable (MAIR index 0):
    //   [4:2] = 0b000 → AttrIndx = 0
    //   [9:8] = 0b11  → SH = inner-shareable
    let normal: u64 = 0b01 | (0b000 << 2) | (0b11 << 8) | (1 << 10); // 0x701

    // ── Populate L1 table ─────────────────────────────────────────────────────

    // Blocks 0–7: Normal memory (0 to 8 GiB).
    // Covers all RAM on QEMU virt and both RPi 5 variants (4 GiB and 8 GiB).
    for i in 0..8usize {
        ID_L1.0[i] = ((i as u64) << 30) | normal;
    }

    // Block 65: Device memory (65 to 66 GiB) — RPi 5 only.
    // Covers RP1 UART0 (0x107D_0010_00), GICD (0x107F_FF90_00),
    // and GICC (0x107F_FFA0_00), which all fall within this 1 GiB window.
    //
    // Device nGnRnE non-shareable (MAIR index 1):
    //   [4:2] = 0b001 → AttrIndx = 1
    //   [9:8] = 0b00  → SH = non-shareable (required for Device memory)
    #[cfg(feature = "rpi5")]
    {
        let device: u64 = 0b01 | (0b001 << 2) | (0b00 << 8) | (1 << 10); // 0x405
        ID_L1.0[65] = (65u64 << 30) | device;
    }

    // L0 entry 0 → L1 table (covers VA 0 … 511 GiB).
    let l1_phys = core::ptr::addr_of!(ID_L1) as u64;
    ID_L0.0[0] = l1_phys | (1 << 1) | (1 << 0); // TABLE | VALID

    let l0_phys = core::ptr::addr_of!(ID_L0) as u64;

    // ── TCR_EL1 ──────────────────────────────────────────────────────────────
    // T0SZ  = 16   → 48-bit VA space (2^48 bytes)
    // TG0   = 00   → 4 KiB granule
    // IRGN0 = 01   → inner write-back, read/write-allocate
    // ORGN0 = 01   → outer write-back, read/write-allocate
    // SH0   = 11   → inner-shareable
    // EPD1  = 1    → disable TTBR1 walks (no kernel high-half yet)
    // IPS   = 010  → 40-bit intermediate PA (1 TiB).
    //                RPi 5 MMIO at ~66 GiB requires > 36 bits.
    let tcr: u64 = 16
        | (0b01  << 8)          // IRGN0
        | (0b01  << 10)         // ORGN0
        | (0b11  << 12)         // SH0
        | (1     << 23)         // EPD1
        | (0b010u64 << 32);     // IPS = 40-bit

    // ── Activate ─────────────────────────────────────────────────────────────

    core::arch::asm!("msr TCR_EL1, {}", in(reg) tcr, options(nostack));
    core::arch::asm!("isb", options(nostack, nomem));

    core::arch::asm!("msr TTBR0_EL1, {}", in(reg) l0_phys, options(nostack));
    core::arch::asm!("isb", options(nostack, nomem));

    // Invalidate all EL1 TLB entries before enabling the MMU.
    core::arch::asm!("tlbi vmalle1", options(nostack, nomem));
    core::arch::asm!("dsb sy",       options(nostack, nomem));
    core::arch::asm!("isb",          options(nostack, nomem));

    // Enable MMU (bit 0), D-cache (bit 2), I-cache (bit 12).
    let mut s: u64;
    core::arch::asm!("mrs {}, SCTLR_EL1", out(reg) s, options(nostack, nomem));
    s |= (1 << 0) | (1 << 2) | (1 << 12);
    core::arch::asm!("msr SCTLR_EL1, {}", in(reg) s, options(nostack));
    core::arch::asm!("isb", options(nostack, nomem));
}
