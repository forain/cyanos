//! Cooperative scheduler — context switching, task lifecycle, IPC blocking.
//!
//! Design: single-CPU, cooperative.  Tasks run until they call `yield_now()`,
//! `block_on()`, or `exit()`.  A static idle context in `run()` is the
//! "scheduler thread" that picks the next ready task on each wake-up.
//!
//! Analogues: Linux kernel/sched/core.c (`schedule`, `switch_to`).

#![no_std]

pub mod context;
pub mod runqueue;
pub mod task;

use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;
use task::{Pid, Task, TaskState};
use context::CpuContext;
use runqueue::RunQueue;

static RUN_QUEUE:    Mutex<RunQueue> = Mutex::new(RunQueue::new());
static NEXT_PID:     Mutex<Pid>     = Mutex::new(1);
static TIMER_TICKS:  AtomicU64      = AtomicU64::new(0);

// ── Single-CPU scheduler state ────────────────────────────────────────────
// Touched only while the single CPU is in scheduler context — no lock needed.

/// Saved register state for the scheduler idle loop.
static mut SCHEDULER_CTX: CpuContext = CpuContext::zeroed();

/// Raw pointer to the currently-running task's `CpuContext`.
/// Non-null only while a task is active on the CPU.
static mut CURRENT_CTX: *mut CpuContext = core::ptr::null_mut();

/// PID of the currently-running task (0 = scheduler idle).
static mut CURRENT_PID: Pid = 0;

// ── Public API ────────────────────────────────────────────────────────────

/// Initialise the scheduler.  Called once from `kernel_main`.
pub fn init() {
    // RunQueue is statically initialised; nothing to do here yet.
}

/// Spawn a new kernel-mode task.
///
/// `entry` must be a `fn() -> !`; the task runs until it calls `exit()`.
/// Returns the new task's PID, or `None` if the run queue is full.
pub fn spawn(entry: fn() -> !, priority: i8) -> Option<Pid> {
    // Allocate an 8 KiB kernel stack (order 1 = 2 × PAGE_SIZE).
    let stack_base = mm::buddy::alloc(1)?;
    let stack_size = mm::buddy::PAGE_SIZE * 2;

    // Zero the stack — alloc returns a physical address; with no MMU (AArch64)
    // or identity mapping (x86-64) this is directly writeable.
    unsafe { (stack_base as *mut u8).write_bytes(0, stack_size); }

    let pid  = alloc_pid();
    let mut t = Task::new_kernel(pid, entry as usize, stack_base, stack_size, 0);
    t.priority = priority;

    if RUN_QUEUE.lock().enqueue(t) {
        Some(pid)
    } else {
        // TODO: free stack on failure
        None
    }
}

/// Enter the scheduler run loop.  Never returns.
pub fn run() -> ! {
    loop {
        let maybe_idx = { RUN_QUEUE.lock().pick_next() };

        if let Some(idx) = maybe_idx {
            // Grab a raw pointer to the task's context and its PID,
            // then drop the lock before switching.
            let (ctx_ptr, pid) = {
                let mut rq = RUN_QUEUE.lock();
                let t = rq.get_mut(idx).unwrap();
                (&mut t.ctx as *mut CpuContext, t.pid)
            };

            unsafe {
                CURRENT_CTX = ctx_ptr;
                CURRENT_PID = pid;
                // Switch to the task.  Returns here when the task yields back.
                context::cpu_switch_to(
                    core::ptr::addr_of_mut!(SCHEDULER_CTX),
                    ctx_ptr as *const CpuContext,
                );
                CURRENT_CTX = core::ptr::null_mut();
                CURRENT_PID = 0;
            }

            // Task yielded: reset to Ready if it hasn't changed state itself
            // (block_on/exit set the state before switching back).
            {
                let mut rq = RUN_QUEUE.lock();
                if let Some(t) = rq.get_mut(idx) {
                    if t.state == TaskState::Running {
                        t.state = TaskState::Ready;
                    }
                }
            }
        } else {
            // No runnable task — wait for an interrupt to make one ready.
            core::hint::spin_loop();
        }
    }
}

/// Voluntarily yield the rest of this task's time-slice.
///
/// The task remains Ready and will be scheduled again on the next pass.
pub fn yield_now() {
    unsafe {
        let ctx = CURRENT_CTX;
        if !ctx.is_null() {
            context::cpu_switch_to(ctx, core::ptr::addr_of!(SCHEDULER_CTX));
        }
    }
}

/// Block the current task until a message is sent to `port`.
///
/// The task's state is set to `Blocked` before switching to the scheduler.
/// `ipc::port::send` calls `unblock_port` to wake it.
pub fn block_on(port: u32) {
    unsafe {
        let pid = CURRENT_PID;
        { RUN_QUEUE.lock().block_on_port(pid, port); }
        let ctx = CURRENT_CTX;
        if !ctx.is_null() {
            context::cpu_switch_to(ctx, core::ptr::addr_of!(SCHEDULER_CTX));
        }
    }
}

/// Wake all tasks that are blocked on `port`.
///
/// Called by `ipc::port::send` after enqueueing a message.
pub fn unblock_port(port: u32) {
    RUN_QUEUE.lock().unblock_port(port);
}

/// Terminate the current task with the given exit code.
pub fn exit(code: i32) -> ! {
    let _ = code;
    unsafe {
        let pid = CURRENT_PID;
        { RUN_QUEUE.lock().mark_zombie(pid); }
        let ctx = CURRENT_CTX;
        if !ctx.is_null() {
            context::cpu_switch_to(ctx, core::ptr::addr_of!(SCHEDULER_CTX));
        }
    }
    // Unreachable: the scheduler will never switch back to a Zombie task.
    loop { core::hint::spin_loop(); }
}

/// Called from the timer ISR on every hardware tick.
///
/// Uses only atomics — safe to call from interrupt context without locks.
/// Currently increments the tick counter and may be extended to set a
/// preemption flag or wake sleeping tasks.
pub fn timer_tick_irq() {
    TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Return the number of timer ticks elapsed since boot.
#[inline]
pub fn ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}

/// Spawn a user-mode task (AArch64 only).
///
/// Allocates an 8 KiB kernel stack, builds the `ret_to_user` frame, and
/// enqueues the task as Ready.  Returns the new PID on success.
#[cfg(target_arch = "aarch64")]
pub fn spawn_user(user_entry: usize, user_stack_top: usize, priority: i8) -> Option<Pid> {
    let stack_base = mm::buddy::alloc(1)?;
    let stack_size = mm::buddy::PAGE_SIZE * 2;
    unsafe { (stack_base as *mut u8).write_bytes(0, stack_size); }

    let pid             = alloc_pid();
    let kernel_stack_top = stack_base + stack_size;
    let ctx = CpuContext::new_user_task(user_entry, user_stack_top, kernel_stack_top);

    let mut t = Task::new_kernel(pid, 0, stack_base, stack_size, 0);
    t.ctx      = ctx;
    t.priority = priority;

    if RUN_QUEUE.lock().enqueue(t) { Some(pid) } else { None }
}

/// Backward-compatible alias used by the syscall dispatch table.
#[inline]
pub fn r#yield() { yield_now(); }

/// Allocate the next available PID.
pub fn alloc_pid() -> Pid {
    let mut n = NEXT_PID.lock();
    let p = *n;
    *n += 1;
    p
}
