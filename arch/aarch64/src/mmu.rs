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

/// Debug memory attributes for a given virtual address
///
/// This performs a page table walk and prints detailed information about
/// the memory mapping, attributes, and permissions for the given address.
pub unsafe fn debug_memory_attributes(addr: usize) {
    extern "C" { fn arch_serial_putc(b: u8); }

    let msg = b"[CYANOS] MEMORY DEBUG FOR ADDRESS: ";
    for &b in msg { arch_serial_putc(b); }
    print_hex_addr(addr as u64);

    // Check if MMU is enabled
    let sctlr: u64;
    core::arch::asm!("mrs {v}, SCTLR_EL1", v = out(reg) sctlr, options(nostack, nomem));
    if sctlr & 1 == 0 {
        let msg = b"MMU disabled - identity mapping\r\n";
        for &b in msg { arch_serial_putc(b); }
        return;
    }

    // Get current page table base
    let ttbr0: u64;
    core::arch::asm!("mrs {}, TTBR0_EL1", out(reg) ttbr0, options(nostack, nomem));

    let msg = b"TTBR0_EL1: ";
    for &b in msg { arch_serial_putc(b); }
    print_hex_addr(ttbr0);

    // Extract address components for page table walk
    let va = addr as u64;
    let l0_index = (va >> 39) & 0x1FF;  // bits 47:39
    let l1_index = (va >> 30) & 0x1FF;  // bits 38:30
    let l2_index = (va >> 21) & 0x1FF;  // bits 29:21
    let l3_index = (va >> 12) & 0x1FF;  // bits 20:12

    let msg = b"Page table indices:\r\n";
    for &b in msg { arch_serial_putc(b); }

    let msg = b"  L0 index: ";
    for &b in msg { arch_serial_putc(b); }
    print_dec_value(l0_index);

    let msg = b"  L1 index: ";
    for &b in msg { arch_serial_putc(b); }
    print_dec_value(l1_index);

    let msg = b"  L2 index: ";
    for &b in msg { arch_serial_putc(b); }
    print_dec_value(l2_index);

    let msg = b"  L3 index: ";
    for &b in msg { arch_serial_putc(b); }
    print_dec_value(l3_index);

    // Walk the page table
    let l0_table = ttbr0 as *const u64;

    if l0_index >= 512 {
        let msg = b"L0 index out of range\r\n";
        for &b in msg { arch_serial_putc(b); }
        return;
    }

    let l0_entry = l0_table.add(l0_index as usize).read_volatile();
    let msg = b"L0 entry: ";
    for &b in msg { arch_serial_putc(b); }
    print_hex_addr(l0_entry);

    if l0_entry & 1 == 0 {
        let msg = b"L0 entry invalid (not present)\r\n";
        for &b in msg { arch_serial_putc(b); }
        return;
    }

    let entry_type = (l0_entry >> 1) & 1;
    if entry_type == 0 {
        let msg = b"L0 entry is a block descriptor (unexpected)\r\n";
        for &b in msg { arch_serial_putc(b); }
        return;
    }

    // L0 points to L1 table
    let l1_table_addr = l0_entry & 0xFFFFFFFFF000;  // Extract bits 47:12
    let l1_table = l1_table_addr as *const u64;

    if l1_index >= 512 {
        let msg = b"L1 index out of range\r\n";
        for &b in msg { arch_serial_putc(b); }
        return;
    }

    let l1_entry = l1_table.add(l1_index as usize).read_volatile();
    let msg = b"L1 entry: ";
    for &b in msg { arch_serial_putc(b); }
    print_hex_addr(l1_entry);

    if l1_entry & 1 == 0 {
        let msg = b"L1 entry invalid (not present)\r\n";
        for &b in msg { arch_serial_putc(b); }
        return;
    }

    let entry_type = (l1_entry >> 1) & 1;
    if entry_type == 0 {
        // L1 block descriptor (1 GiB block)
        let msg = b"L1 block descriptor (1 GiB block)\r\n";
        for &b in msg { arch_serial_putc(b); }
        decode_block_attributes(l1_entry);
        return;
    }

    let msg = b"L1 table descriptor - continuing to L2\r\n";
    for &b in msg { arch_serial_putc(b); }

    // L1 points to L2 table (not implemented in our simple identity mapping)
    let msg = b"L2/L3 table walk not implemented (we use 1 GiB blocks)\r\n";
    for &b in msg { arch_serial_putc(b); }
}

/// Decode and print block/page descriptor attributes
unsafe fn decode_block_attributes(entry: u64) {
    extern "C" { fn arch_serial_putc(b: u8); }

    let msg = b"Block attributes:\r\n";
    for &b in msg { arch_serial_putc(b); }

    // Access flag (bit 10)
    let af = (entry >> 10) & 1;
    if af == 1 {
        let msg = b"  AF=1 (accessed)\r\n";
        for &b in msg { arch_serial_putc(b); }
    } else {
        let msg = b"  AF=0 (not accessed)\r\n";
        for &b in msg { arch_serial_putc(b); }
    }

    // Shareability (bits 9:8)
    let sh = (entry >> 8) & 3;
    let sh_prefix = b"  SH=0x";
    for &b in sh_prefix { arch_serial_putc(b); }
    let sh_digit = b'0' + sh as u8;
    arch_serial_putc(sh_digit);

    if sh == 0b00 {
        let msg = b" (non-shareable)\r\n";
        for &b in msg { arch_serial_putc(b); }
    } else if sh == 0b01 {
        let msg = b" (reserved)\r\n";
        for &b in msg { arch_serial_putc(b); }
    } else if sh == 0b10 {
        let msg = b" (outer shareable)\r\n";
        for &b in msg { arch_serial_putc(b); }
    } else if sh == 0b11 {
        let msg = b" (inner shareable)\r\n";
        for &b in msg { arch_serial_putc(b); }
    }

    // Access permissions (bits 7:6)
    let ap = (entry >> 6) & 3;
    let ap_prefix = b"  AP=0x";
    for &b in ap_prefix { arch_serial_putc(b); }
    let ap_digit = b'0' + ap as u8;
    arch_serial_putc(ap_digit);

    if ap == 0b00 {
        let msg = b" (EL1 RW, EL0 no access)\r\n";
        for &b in msg { arch_serial_putc(b); }
    } else if ap == 0b01 {
        let msg = b" (EL1 RW, EL0 RW)\r\n";
        for &b in msg { arch_serial_putc(b); }
    } else if ap == 0b10 {
        let msg = b" (EL1 RO, EL0 no access)\r\n";
        for &b in msg { arch_serial_putc(b); }
    } else if ap == 0b11 {
        let msg = b" (EL1 RO, EL0 RO)\r\n";
        for &b in msg { arch_serial_putc(b); }
    }

    // Memory attribute index (bits 4:2)
    let attr_indx = (entry >> 2) & 7;
    let msg = b"  AttrIndx=";
    for &b in msg { arch_serial_putc(b); }
    print_dec_value(attr_indx);

    if attr_indx == 0 {
        let msg = b"    (Normal WB/WA)\r\n";
        for &b in msg { arch_serial_putc(b); }
    } else if attr_indx == 1 {
        let msg = b"    (Device nGnRnE)\r\n";
        for &b in msg { arch_serial_putc(b); }
    } else {
        let msg = b"    (Other/Unknown)\r\n";
        for &b in msg { arch_serial_putc(b); }
    }

    // Execute permissions
    let uxn = (entry >> 54) & 1;  // User Execute Never
    let pxn = (entry >> 53) & 1;  // Privileged Execute Never

    if uxn == 1 {
        let msg = b"  UXN=1 (user execute never)\r\n";
        for &b in msg { arch_serial_putc(b); }
    } else {
        let msg = b"  UXN=0 (user execute allowed)\r\n";
        for &b in msg { arch_serial_putc(b); }
    }

    if pxn == 1 {
        let msg = b"  PXN=1 (privileged execute never)\r\n";
        for &b in msg { arch_serial_putc(b); }
    } else {
        let msg = b"  PXN=0 (privileged execute allowed)\r\n";
        for &b in msg { arch_serial_putc(b); }
    }

    // Physical address (block base)
    let block_addr = entry & 0xFFFFC0000000;  // Bits 47:30 for 1 GiB blocks
    let msg = b"  Physical block base: ";
    for &b in msg { arch_serial_putc(b); }
    print_hex_addr(block_addr);
}

/// Print a hex address value to serial
unsafe fn print_hex_addr(value: u64) {
    extern "C" { fn arch_serial_putc(b: u8); }

    arch_serial_putc(b'0');
    arch_serial_putc(b'x');

    for i in (0..16).rev() {
        let nibble = ((value >> (i * 4)) & 0xF) as u8;
        let c = if nibble < 10 { b'0' + nibble } else { b'A' + nibble - 10 };
        arch_serial_putc(c);
    }

    arch_serial_putc(b'\r');
    arch_serial_putc(b'\n');
}

/// Print a decimal value to serial
unsafe fn print_dec_value(value: u64) {
    extern "C" { fn arch_serial_putc(b: u8); }

    if value == 0 {
        arch_serial_putc(b'0');
    } else {
        let mut digits = [0u8; 20];
        let mut num = value;
        let mut i = 0;

        while num > 0 {
            digits[i] = (num % 10) as u8 + b'0';
            num /= 10;
            i += 1;
        }

        for j in (0..i).rev() {
            arch_serial_putc(digits[j]);
        }
    }

    arch_serial_putc(b'\r');
    arch_serial_putc(b'\n');
}

/// C-compatible wrapper for debug_memory_attributes
#[no_mangle]
pub unsafe extern "C" fn debug_memory_attributes_aarch64(addr: usize) {
    debug_memory_attributes(addr);
}
