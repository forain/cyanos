//! Scheduler — preemptive, priority-based task scheduler.
//!
//! Inspired by Linux CFS but simplified for a microkernel where most work
//! is IPC-driven, not CPU-bound.

#![no_std]

pub mod task;
pub mod runqueue;

use spin::Mutex;
use task::Pid;
use runqueue::RunQueue;

static RUN_QUEUE: Mutex<RunQueue> = Mutex::new(RunQueue::new());
static NEXT_PID: spin::Mutex<Pid> = spin::Mutex::new(1);

/// Initialise the scheduler. Called once from `kernel_main`.
pub fn init() {
    // Nothing to do yet; RunQueue is statically initialised.
}

/// Enter the scheduler loop — never returns.
pub fn run() -> ! {
    loop {
        let maybe_idx = {
            let mut rq = RUN_QUEUE.lock();
            rq.pick_next()
        };
        if let Some(_idx) = maybe_idx {
            // TODO: context-switch to task at _idx.
        } else {
            // CPU idle — halt until next interrupt.
            core::hint::spin_loop();
        }
    }
}

/// Yield the current task's remaining timeslice.
pub fn r#yield() {
    // TODO: trigger reschedule.
}

/// Terminate the current task with the given exit code.
pub fn exit(code: i32) -> ! {
    let _ = code;
    // TODO: clean up task, wake any waiters, schedule next.
    loop { core::hint::spin_loop(); }
}

/// Allocate the next available PID.
pub fn alloc_pid() -> Pid {
    let mut pid = NEXT_PID.lock();
    let p = *pid;
    *pid += 1;
    p
}
