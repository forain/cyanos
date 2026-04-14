//! Cyanos kernel entry point.
//!
//! `kernel_main` is called by the arch-specific `_start` stub (entry_*.s)
//! after the stack is set up and BSS is zeroed.  It receives the physical
//! address of the boot information structure (MBI for x86-64, DTB for AArch64).

#![no_std]
#![no_main]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

use core::panic::PanicInfo;

mod init;
mod syscall;

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
    // QEMU virt UART0 is PL011 at 0x09000000.
    // CR register: enable TX (bit 8) + enable UART (bit 0).
    let base = 0x09000000usize;
    let cr = (base + 0x30) as *mut u32;
    cr.write_volatile(0x0301); // UARTEN | TXE
}

unsafe fn serial_write_byte(b: u8) {
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
        let base = 0x09000000usize;
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
pub extern "C" fn kernel_main(boot_info_addr: usize) -> ! {
    // 1. Initialise serial UART as early as possible (needed by panic handler).
    unsafe { early_serial_init(); }
    serial_print("\n[CYANOS] kernel_main — starting\n");

    // 2. Initialise architecture-specific hardware (GDT/IDT on x86, vectors on AArch64).
    #[cfg(target_arch = "x86_64")]
    { arch_x86_64::init(); }
    #[cfg(target_arch = "aarch64")]
    { arch_aarch64::init(); }

    // 3. Parse boot information into a unified BootInfo.
    let boot_info = unsafe {
        #[cfg(target_arch = "x86_64")]
        { boot::multiboot2::parse(boot_info_addr) }
        #[cfg(target_arch = "aarch64")]
        { boot::device_tree::parse(boot_info_addr) }
    };

    serial_print("[CYANOS] memory map: ");
    print_hex(boot_info.total_available_memory());
    serial_print(" bytes available\n");

    // 4. Initialise the physical memory manager from the memory map.
    mm::init_with_map(boot_info.memory_regions());

    // 5. Initialise the scheduler.
    sched::init();

    // 6. Initialise the IPC subsystem.
    ipc::init();

    // 7. Spawn PID-1 init task.
    match sched::spawn(init::init_task_main, 0) {
        Some(pid) => {
            serial_print("[CYANOS] init task spawned, PID ");
            print_hex(pid as u64);
            serial_print("\n");
        }
        None => panic!("kernel_main: failed to spawn init task"),
    }

    serial_print("[CYANOS] subsystems initialised — entering scheduler\n");

    // 8. Hand off to the scheduler.  Never returns.
    sched::run()
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
