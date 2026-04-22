//! Cyanos kernel entry point.
//!
//! `kernel_main` is called by the arch-specific `_start` stub (entry_*.s)
//! after the stack is set up and BSS is zeroed.  It receives the physical
//! address of the boot information structure (MBI for x86-64, DTB for AArch64).

#![no_std]
#![no_main]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![cfg_attr(target_arch = "x86_64", feature(sync_unsafe_cell))]

use core::panic::PanicInfo;
use core::alloc::{GlobalAlloc, Layout};

// ── Global Allocator ─────────────────────────────────────────────────────────

struct DummyAllocator;

unsafe impl GlobalAlloc for DummyAllocator {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        core::ptr::null_mut()
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Do nothing - this is a dummy allocator
    }
}

#[global_allocator]
static ALLOCATOR: DummyAllocator = DummyAllocator;

// External crate imports
#[cfg(target_arch = "x86_64")]
extern crate arch_x86_64;
#[cfg(target_arch = "aarch64")]
extern crate arch_aarch64;

mod init;
mod syscall;
mod mem;

// ── Architecture-specific boot stubs ─────────────────────────────────────────
// Each stub provides `_start`, sets up the stack, zeros BSS, then calls
// `kernel_main(boot_info_addr: usize)`.

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("entry_aarch64.s"));

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("entry_x86_64.s"));

// ── Serial port (for early debug output) ─────────────────────────────────────

#[cfg(target_arch = "x86_64")]
unsafe fn early_serial_init() {
    // 16550 UART at COM1 (0x3F8), 115200 8N1.
    use core::arch::asm;
    macro_rules! outb {
        ($port:expr, $val:expr) => {
            asm!("out dx, al", in("dx") $port as u16, in("al") $val as u8,
                 options(nomem, nostack));
        }
    }
    outb!(0x3F9, 0x00u8); // disable interrupts
    outb!(0x3FB, 0x80u8); // enable DLAB
    outb!(0x3F8, 0x01u8); // baud divisor lo (115200)
    outb!(0x3F9, 0x00u8); // baud divisor hi
    outb!(0x3FB, 0x03u8); // 8N1
    outb!(0x3FA, 0xC7u8); // FIFO enable
}

#[cfg(target_arch = "aarch64")]
unsafe fn early_serial_init() {
    // UART is already working from assembly code, skip MMIO config for now
    // to avoid hanging on PL011 register access before MMU is set up
}

/// Non-blocking serial RX poll.  Returns `Some(byte)` if a character is
/// waiting in the UART RX FIFO, or `None` if the FIFO is empty.
///
/// AArch64 (PL011): FR bit 4 = RXFE (RX FIFO empty); read from DR.
/// x86-64 (16550):  LSR bit 0 = DR (Data Ready);       read from RHR (0x3F8).
pub fn serial_read_byte() -> Option<u8> {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use core::arch::asm;
        let mut status: u8;
        asm!("in al, dx", out("al") status, in("dx") 0x3FDu16, options(nomem, nostack));
        if status & 0x01 != 0 {
            let mut b: u8;
            asm!("in al, dx", out("al") b, in("dx") 0x3F8u16, options(nomem, nostack));
            return Some(b);
        }
        return None;
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        #[cfg(not(feature = "rpi5"))]
        let base = 0x09000000usize;
        #[cfg(feature = "rpi5")]
        let base = 0x107D_0010_00usize;
        let fr = (base + 0x18) as *const u32;
        // bit 4 = RXFE (RX FIFO empty); if set there is no data
        if fr.read_volatile() & (1 << 4) != 0 { return None; }
        let dr = base as *const u32;
        Some((dr.read_volatile() & 0xFF) as u8)
    }
}

pub unsafe fn serial_write_byte(b: u8) {
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::asm;
        // Wait until THRE (Transmit Holding Register Empty) is set.
        loop {
            let mut status: u8;
            asm!("in al, dx", out("al") status, in("dx") 0x3FDu16,
                 options(nomem, nostack));
            if status & 0x20 != 0 { break; }
        }
        asm!("out dx, al", in("dx") 0x3F8u16, in("al") b,
             options(nomem, nostack));
    }
    #[cfg(target_arch = "aarch64")]
    {
        // Select UART base to match the board compiled for.
        #[cfg(not(feature = "rpi5"))]
        let base = 0x09000000usize;       // QEMU virt PL011
        #[cfg(feature = "rpi5")]
        let base = 0x107D_0010_00usize;   // RPi5 RP1 PL011
        // Wait until TX FIFO not full (FR register bit 5 = TXFF).
        let fr = (base + 0x18) as *const u32;
        while fr.read_volatile() & (1 << 5) != 0 {}
        let dr = base as *mut u32;
        dr.write_volatile(b as u32);
    }
}

pub fn serial_print(s: &str) {
    for b in s.bytes() {
        unsafe { serial_write_byte(b); }
        if b == b'\n' { unsafe { serial_write_byte(b'\r'); } }
    }
}

/// Write raw bytes to serial without any LF→CRLF translation.
/// Used by `sys_write` for stdout/stderr (fd 1/2).
pub fn serial_write_raw(buf: &[u8]) {
    for &b in buf { unsafe { serial_write_byte(b); } }
}

/// Export for C calling convention from scheduler debugging
#[no_mangle]
pub extern "C" fn serial_print_bytes(ptr: *const u8, len: usize) {
    if ptr.is_null() { return; }
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    serial_write_raw(bytes);
}

// Simple hex formatter for panic messages — no alloc needed.
fn print_hex(mut n: u64) {
    serial_print("0x");
    let mut buf = [0u8; 16];
    let mut i = 16;
    if n == 0 {
        serial_print("0");
        return;
    }
    while n > 0 {
        i -= 1;
        buf[i] = b"0123456789abcdef"[(n & 0xF) as usize];
        n >>= 4;
    }
    for &c in &buf[i..] {
        unsafe { serial_write_byte(c); }
    }
}

// ── Kernel main ───────────────────────────────────────────────────────────────

/// Called by `_start` after stack setup and BSS zeroing.
///
/// `boot_info_addr`:
///   x86-64  — physical address of the multiboot2 info structure
///   AArch64 — physical address of the device tree blob (DTB), or 0
#[no_mangle]
pub extern "C" fn kernel_main(_boot_info_addr: usize) -> ! {
    // Output early debug marker to show we've reached kernel_main
    #[cfg(target_arch = "x86_64")]
    unsafe {
        // Write '@' to COM1 to show kernel_main entry
        let mut status: u8;
        // Wait for transmitter ready
        loop {
            core::arch::asm!("in al, dx", out("al") status, in("dx") 0x3FDu16, options(nomem, nostack));
            if status & 0x20 != 0 { break; }
        }
        core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'@', options(nomem, nostack));
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        // Write '@' to UART to show kernel_main entry
        #[cfg(not(feature = "rpi5"))]
        let base = 0x09000000usize;
        #[cfg(feature = "rpi5")]
        let base = 0x107D_0010_00usize;

        // Wait until TX FIFO not full (FR register bit 5 = TXFF).
        let fr = (base + 0x18) as *const u32;
        while fr.read_volatile() & (1 << 5) != 0 {}
        let dr = base as *mut u32;
        dr.write_volatile(b'@' as u32);
    }

    // Minimal kernel - just halt immediately
    loop {
        core::hint::spin_loop();
    }
}

// ── Test functions ────────────────────────────────────────────────────────────

fn test_kernel_task() -> ! {
    serial_print("[TASK] Simple kernel task started successfully!\n");
    serial_print("[TASK] This proves task spawning works\n");

    // Simple loop to show the task is running
    for i in 0..3 {
        serial_print("[TASK] Iteration: ");
        print_number(i);
        serial_print("\n");

        // Small delay
        for _ in 0..500000 {
            core::hint::spin_loop();
        }
    }

    serial_print("[TASK] Test kernel task completing\n");
    sched::exit(0);
}

fn print_pid(pid: u32) {
    print_number(pid);
}

fn print_number(mut n: u32) {
    if n == 0 {
        serial_print("0");
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 10;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + ((n % 10) as u8);
        n /= 10;
    }
    let s = unsafe { core::str::from_utf8_unchecked(&buf[i..]) };
    serial_print(s);
}

// ── Panic handler ─────────────────────────────────────────────────────────────

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_print("\n[CYANOS] KERNEL PANIC");

    if let Some(loc) = info.location() {
        serial_print(" at ");
        serial_print(loc.file());
        serial_print(":");
        print_hex(loc.line() as u64);
    }

    if let Some(msg) = info.message().as_str() {
        serial_print(": ");
        serial_print(msg);
    }

    serial_print("\n[CYANOS] halted.\n");

    loop {
        core::hint::spin_loop();
    }
}
