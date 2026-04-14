//! PID-1 init task — first process after the kernel bootstraps.
//!
//! In a microkernel all "system" work lives in user-space servers that init
//! spawns and supervises.  For now this is a kernel-mode task that exercises
//! context switching and IPC before parking in a yield loop.

use crate::serial_print;

/// Entry point for the init task.  Must never return (`fn() -> !`).
pub fn init_task_main() -> ! {
    serial_print("[LOS] init: task started (PID 1)\n");

    // ── IPC smoke test ────────────────────────────────────────────────────────
    // Allocate a port, send one message to ourselves, receive it back.
    // This exercises the full send → unblock → recv path.
    match ipc::port::create(1) {
        Some(port) => {
            ipc::port::send(port, ipc::Message::empty());

            // recv() is non-blocking; the message was just enqueued so it is
            // immediately available.
            match ipc::port::recv(port) {
                Some(_) => serial_print("[LOS] init: IPC loopback OK\n"),
                None    => serial_print("[LOS] init: IPC loopback FAILED — no message\n"),
            }

            // Enter a blocking event loop: wait for more messages.
            // (Nothing else sends to us right now, so we just yield forever.)
            serial_print("[LOS] init: entering event loop\n");
            loop {
                match ipc::port::recv(port) {
                    Some(_msg) => {
                        serial_print("[LOS] init: message received\n");
                    }
                    None => {
                        // No message — block until something sends to our port.
                        sched::block_on(port);
                    }
                }
            }
        }
        None => {
            serial_print("[LOS] init: IPC port alloc failed — yielding forever\n");
            loop { sched::yield_now(); }
        }
    }
}
