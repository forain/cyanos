//! PID-1 init task — first process after the kernel bootstraps.
//!
//! In a microkernel all "system" work lives in user-space servers that init
//! spawns and supervises.  For now this is a kernel-mode task that exercises
//! context switching and IPC before parking in a yield loop.

use crate::serial_print;

/// Entry point for the init task.  Must never return (`fn() -> !`).
pub fn init_task_main() -> ! {
    serial_print("[CYANOS] init: task started (PID 1)\n");

    // ── IPC smoke test ────────────────────────────────────────────────────────
    // Allocate a port, send one message to ourselves, receive it back.
    // This exercises the full send → unblock → recv path.
    match ipc::port::create(1) {
        Some(port) => {
            ipc::port::send(port, ipc::Message::empty());

            // recv() is non-blocking; the message was just enqueued so it is
            // immediately available.
            match ipc::port::recv(port) {
                Some(_) => serial_print("[CYANOS] init: IPC loopback OK\n"),
                None    => serial_print("[CYANOS] init: IPC loopback FAILED — no message\n"),
            }

            // Enter an event loop: service messages and print a heartbeat every
            // 100 ticks (≈1 s at 100 Hz) so we can confirm the timer fires.
            serial_print("[CYANOS] init: entering event loop\n");
            let mut last_heartbeat: u64 = 0;
            loop {
                // Service all pending messages without blocking.
                while let Some(_msg) = ipc::port::recv(port) {
                    serial_print("[CYANOS] init: message received\n");
                }

                // Heartbeat: print once per second.
                let t = sched::ticks();
                if t.wrapping_sub(last_heartbeat) >= 100 {
                    last_heartbeat = t;
                    serial_print("[CYANOS] init: heartbeat\n");
                }

                // Yield rather than hard-block so the tick check above can fire.
                sched::yield_now();
            }
        }
        None => {
            serial_print("[CYANOS] init: IPC port alloc failed — yielding forever\n");
            loop { sched::yield_now(); }
        }
    }
}
