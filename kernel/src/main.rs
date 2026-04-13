//! LOS Kernel — microkernel entry point.
//!
//! Architecture mirrors Linux's separation of concerns but enforces
//! isolation: drivers and services run in user-space servers; only the
//! minimal nucleus (memory, scheduling, IPC) lives here.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

use core::panic::PanicInfo;

mod syscall;

/// Kernel entry point called by the bootloader after basic hardware init.
#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // 1. Initialise the physical memory manager.
    mm::init();

    // 2. Initialise the scheduler.
    sched::init();

    // 3. Initialise the IPC subsystem.
    ipc::init();

    // 4. Spawn the init server (PID 1).
    // sched::spawn_init();

    // 5. Hand off to the scheduler — never returns.
    sched::run()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // In a real kernel, emit the panic over a serial port / framebuffer.
    let _ = info;
    loop {
        core::hint::spin_loop();
    }
}
