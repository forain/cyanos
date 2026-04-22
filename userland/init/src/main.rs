//! CyanOS Init - userspace init program (PID 1)
//!
//! This is the first userspace program that runs and manages the system.

#![no_std]
#![no_main]

extern crate cyanos_libc;

use cyanos_libc::{write, STDOUT_FILENO, getpid, exit, fork, execve, sched_yield};

// Embedded shell binary for Phase 1 execve
// static SHELL_BINARY: &[u8] = include_bytes!("../../target/aarch64-unknown-none/release/shell");

/// Called by `__libc_start_main` after the C runtime is set up.
#[no_mangle]
pub unsafe extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    write_str("CyanOS Init (PID 1) starting...\n");

    // Show our PID
    write_str("Init PID: ");
    write_u32(getpid() as u32);
    write_str("\n");

    write_str("Init process running successfully!\n");
    write_str("Testing basic syscalls...\n");

    // Test basic syscalls without fork/execve to isolate scheduler vs memory issues
    for i in 0..5 {
        write_str("Test iteration: ");
        write_u32(i);
        write_str("\n");

        // Test yield
        sched_yield();

        // Small delay loop
        for _ in 0..100000 {
            core::hint::spin_loop();
        }
    }

    write_str("All tests completed successfully!\n");

    // Simple init loop - in a real system this would reap children
    loop {
        write_str("Init: yielding to scheduler\n");
        sched_yield();

        // Small delay
        for _ in 0..1000000 {
            core::hint::spin_loop();
        }
    }
}

unsafe fn write_str(s: &str) {
    write(STDOUT_FILENO, s.as_ptr(), s.len());
}

unsafe fn write_u32(mut n: u32) {
    let mut buf = [0u8; 10];
    if n == 0 {
        write(STDOUT_FILENO, b"0".as_ptr(), 1);
        return;
    }
    let mut i = 10usize;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    write(STDOUT_FILENO, buf.as_ptr().add(i), 10 - i);
}