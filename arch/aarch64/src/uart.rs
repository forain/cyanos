//! PL011 UART driver for AArch64.
//!
//! **QEMU -machine virt** (default):
//!   Base: 0x0900_0000  UARTCLK: 24 MHz → IBRD=13, FBRD=1  (115200 baud)
//!
//! **Raspberry Pi 5 (BCM2712 / RP1)** — enabled by the `rpi5` cargo feature:
//!   Base: 0x107D_0010_00  UARTCLK: 48 MHz → IBRD=26, FBRD=3  (115200 baud)
//!
//! Baud divisor formula:  BAUDDIV = UARTCLK / (16 × baud)
//!   QEMU:  24_000_000 / (16 × 115_200) ≈ 13.0208  → IBRD=13, FBRD=round(0.0208×64)=1
//!   RPi5:  48_000_000 / (16 × 115_200) ≈ 26.0417  → IBRD=26, FBRD=round(0.0417×64)=3

use core::sync::atomic::{AtomicUsize, Ordering};

// ── Board-specific constants ──────────────────────────────────────────────────

/// MMIO base address of the PL011.
#[cfg(not(feature = "rpi5"))]
pub const BASE: usize = 0x0900_0000;       // QEMU virt

#[cfg(feature = "rpi5")]
pub const BASE: usize = 0x107D_0010_00;    // RPi 5 RP1 UART0

/// Integer baud-rate divisor.
#[cfg(not(feature = "rpi5"))]
const IBRD_VAL: u32 = 13;

#[cfg(feature = "rpi5")]
const IBRD_VAL: u32 = 26;

/// Fractional baud-rate divisor.
#[cfg(not(feature = "rpi5"))]
const FBRD_VAL: u32 = 1;

#[cfg(feature = "rpi5")]
const FBRD_VAL: u32 = 3;

// ── Register offsets ──────────────────────────────────────────────────────────
const DR:   usize = 0x000; // Data register (write = TX, read = RX)
const FR:   usize = 0x018; // Flag register
const IBRD: usize = 0x024; // Integer baud-rate divisor
const FBRD: usize = 0x028; // Fractional baud-rate divisor
const LCRH: usize = 0x02C; // Line control register (high)
const CR:   usize = 0x030; // Control register

// ── Flag register bits ────────────────────────────────────────────────────────
const FR_TXFF: u32 = 1 << 5; // TX FIFO full — spin until clear before writing

// ── Initialise the PL011 for 115 200 8N1 with FIFO enabled ───────────────────

/// Initialise the PL011 UART at the compile-time `BASE` address.
///
/// # Safety
/// Must be called from a context where MMIO at `BASE` is accessible
/// (identity-mapped or the MMU is off).
pub unsafe fn init() {
    // Stamp the compile-time base so rd/wr helpers use the correct address
    // even if reinit() was somehow called first (defensive).
    UART_BASE_ADDR.store(BASE, Ordering::Relaxed);

    // Disable UART while programming line-control registers.
    wr(CR, 0);

    wr(IBRD, IBRD_VAL);
    wr(FBRD, FBRD_VAL);

    // LCRH: WLEN = 0b11 (8-bit), FEN = 1 (FIFO enable).
    wr(LCRH, (0b11 << 5) | (1 << 4));

    // CR: UARTEN (bit 0) | TXE (bit 8) | RXE (bit 9).
    wr(CR, (1 << 0) | (1 << 8) | (1 << 9));
}

/// Re-initialise the PL011 at a runtime-discovered base address.
///
/// Called from `kernel_main` when the DTB reports a UART base address
/// different from the compile-time `BASE` constant.  Updates `UART_BASE_ADDR`
/// first so that all subsequent `rd`/`wr` (and `putc`) calls target the new
/// address — including `arch_serial_putc` called from exception handlers.
///
/// # Safety
/// `base` must point to a valid, identity-mapped PL011 MMIO region.
pub unsafe fn reinit(base: usize) {
    // Update the runtime base BEFORE any register access so wr() uses it.
    UART_BASE_ADDR.store(base, Ordering::Relaxed);

    wr(CR,   0);
    wr(IBRD, IBRD_VAL);
    wr(FBRD, FBRD_VAL);
    wr(LCRH, (0b11 << 5) | (1 << 4));
    wr(CR,   (1 << 0) | (1 << 8) | (1 << 9));
}

/// Write one byte to the TX FIFO, spinning until space is available.
///
/// # Safety
/// `init()` must have been called first.
pub unsafe fn putc(c: u8) {
    while rd(FR) & FR_TXFF != 0 {
        core::hint::spin_loop();
    }
    wr(DR, c as u32);
}

// ── Runtime UART base ─────────────────────────────────────────────────────────
//
// Initialised to the compile-time BASE constant; updated by `reinit()` when
// the DTB reports a different address.  Using an AtomicUsize ensures that
// `arch_serial_putc` (called from IRQ/exception context) always sees the
// most recently configured base without needing a lock.

static UART_BASE_ADDR: AtomicUsize = AtomicUsize::new(BASE);

// ── Register helpers ──────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn rd(off: usize) -> u32 {
    let base = UART_BASE_ADDR.load(Ordering::Relaxed);
    ((base + off) as *const u32).read_volatile()
}

#[inline(always)]
unsafe fn wr(off: usize, val: u32) {
    let base = UART_BASE_ADDR.load(Ordering::Relaxed);
    ((base + off) as *mut u32).write_volatile(val);
}

// ── C-callable wrappers (resolved by the drivers crate at link time) ──────────

/// Initialise the PL011 — called from `Serial::probe()` on non-x86 targets.
#[no_mangle]
pub unsafe extern "C" fn arch_serial_init() {
    init();
}

/// Write one byte — called from `Serial::write_byte()` on non-x86 targets.
#[no_mangle]
pub unsafe extern "C" fn arch_serial_putc(c: u8) {
    putc(c);
}
