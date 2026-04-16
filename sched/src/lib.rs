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

use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use spin::Mutex;
use task::{Pid, Task, TaskState};
use context::CpuContext;
use runqueue::RunQueue;

static RUN_QUEUE:       Mutex<RunQueue> = Mutex::new(RunQueue::new());
static NEXT_PID:        Mutex<Pid>      = Mutex::new(1);
static TIMER_TICKS:     AtomicU64       = AtomicU64::new(0);
/// Set by timer_tick_irq; cleared and acted on by preempt_check.
static PREEMPT_NEEDED:  AtomicBool      = AtomicBool::new(false);
/// Optional hook called with a PID just before its task slot is reclaimed.
/// Registered by the IPC layer to release ports owned by the exiting task.
static TASK_EXIT_HOOK:  AtomicPtr<()>   = AtomicPtr::new(core::ptr::null_mut());

// ── Exit-code log ────────────────────────────────────────────────────────────
//
// When the scheduler's run() loop reaps a zombie, the exit code is saved here
// so that a subsequent wait_pid() call can retrieve it even though the task has
// already been removed from the run queue.
//
// Without this, wait_pid() always returns None: by the time it resumes after
// yield_now(), run() has already reaped the zombie and find_pid() finds nothing.
//
// Capacity: MAX_TASKS (256).  When full, the oldest entry is evicted (rotate).
// A task that is waited on with wait_pid() removes its entry via take_exit_code(),
// keeping the log sparse in normal usage.

const EXIT_LOG_LEN: usize = 256;

struct ExitRecord { pid: Pid, code: i32 }

static EXIT_LOG: Mutex<[Option<ExitRecord>; EXIT_LOG_LEN]> =
    Mutex::new([const { None }; EXIT_LOG_LEN]);

/// Record `(pid, code)` in the exit log so that a future `wait_pid` can
/// retrieve the exit code after the task has been removed from the run queue.
fn log_exit(pid: Pid, code: i32) {
    let mut log = EXIT_LOG.lock();
    for slot in log.iter_mut() {
        if slot.is_none() {
            *slot = Some(ExitRecord { pid, code });
            return;
        }
    }
    // Log full — evict the oldest entry (slot 0) and append at the end.
    log.rotate_left(1);
    log[EXIT_LOG_LEN - 1] = Some(ExitRecord { pid, code });
}

/// Remove and return the exit code for `pid` from the log, or `None` if not found.
fn take_exit_code(pid: Pid) -> Option<i32> {
    let mut log = EXIT_LOG.lock();
    for slot in log.iter_mut() {
        if let Some(rec) = slot {
            if rec.pid == pid {
                let code = rec.code;
                *slot = None;
                return Some(code);
            }
        }
    }
    None
}

// ── Per-CPU scheduler state ───────────────────────────────────────────────
//
// Each CPU has its own idle context, current-task pointer, and current PID.
// Indexed by the logical CPU ID returned by arch_cpu_id().

/// Maximum number of CPUs supported.
pub const MAX_CPUS: usize = 8;

// Arch-provided: return the logical CPU index for the calling CPU.
// Must be in the range [0, MAX_CPUS).  BSP = 0; APs = 1, 2, ...
// Implemented by each arch crate and resolved at link time.
extern "C" { fn arch_cpu_id() -> usize; }

#[inline(always)]
fn cpu_id() -> usize {
    // Safety: arch_cpu_id reads a hardware register; clamped for safety.
    unsafe { arch_cpu_id().min(MAX_CPUS - 1) }
}

/// Saved register state for each CPU's scheduler idle loop.
static mut SCHEDULER_CTX: [CpuContext; MAX_CPUS] =
    [const { CpuContext::zeroed() }; MAX_CPUS];

/// Raw pointer to each CPU's currently-running task's `CpuContext`.
/// Non-null only while a task is active on that CPU.
static mut CURRENT_CTX: [*mut CpuContext; MAX_CPUS] =
    [core::ptr::null_mut(); MAX_CPUS];

/// PID of the currently-running task on each CPU (0 = scheduler idle).
static mut CURRENT_PID: [Pid; MAX_CPUS] = [0; MAX_CPUS];

// ── Public API ────────────────────────────────────────────────────────────

/// Initialise the scheduler.  Called once from `kernel_main`.
pub fn init() {
    // RunQueue is statically initialised; nothing to do here yet.
}

/// Register a hook to be called with a PID when a task is about to be reaped.
///
/// Intended for the IPC layer to release ports owned by the exiting task.
/// Only one hook is supported; a second call overwrites the first.
pub fn set_task_exit_hook(f: fn(u32)) {
    TASK_EXIT_HOOK.store(f as *mut (), Ordering::Release);
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
        mm::buddy::free(stack_base, 1);
        None
    }
}

/// Enter the scheduler run loop.  Never returns.
pub fn run() -> ! {
    loop {
        let maybe_idx = { RUN_QUEUE.lock().pick_next() };

        if let Some(idx) = maybe_idx {
            // Grab a raw pointer to the task's context, its PID, and its
            // page table root, then drop the lock before switching.
            let (ctx_ptr, pid, _kernel_stack_top, page_table) = {
                let mut rq = RUN_QUEUE.lock();
                let t = rq.get_mut(idx).unwrap();
                let kst = t.kernel_stack + mm::buddy::PAGE_SIZE * 2;
                (&mut t.ctx as *mut CpuContext, t.pid, kst, t.page_table)
            };

            // Update the per-CPU kernel stack pointer used on exception entry
            // from user space: TSS.rsp0 on x86-64, TPIDR_EL1 on AArch64.
            unsafe { arch_set_kernel_stack(_kernel_stack_top as u64); }

            unsafe {
                let id = cpu_id();
                CURRENT_CTX[id] = ctx_ptr;
                CURRENT_PID[id] = pid;

                // Switch the page-table root for this task (TTBR0_EL1 / CR3).
                arch_set_page_table(page_table);

                // Switch to the task.  Returns here when the task yields back.
                context::cpu_switch_to(
                    core::ptr::addr_of_mut!(SCHEDULER_CTX[id]),
                    ctx_ptr as *const CpuContext,
                );

                // Task yielded back to scheduler — clear the user page table.
                arch_set_page_table(0);

                CURRENT_CTX[id] = core::ptr::null_mut();
                CURRENT_PID[id] = 0;
            }

            // ── Post-switch: update state or reap ────────────────────────────
            //
            // If the task is still Running it yielded voluntarily → mark Ready.
            // If it's Zombie, free its resources now (kernel stack + address space
            // + IPC ports) and remove it from the run queue.
            //
            // zombie_info: (kernel_stack_base, pid, page_table_root, exit_code)
            let zombie_info: Option<(usize, u32, usize, i32)> = {
                let mut rq = RUN_QUEUE.lock();
                if let Some(t) = rq.get_mut(idx) {
                    if t.state == TaskState::Zombie {
                        Some((t.kernel_stack, t.pid, t.page_table, t.exit_code))
                    } else {
                        if t.state == TaskState::Running {
                            t.state = TaskState::Ready;
                        }
                        None
                    }
                } else {
                    None
                }
            };

            if let Some((stack_base, pid, page_table, exit_code)) = zombie_info {
                // Call the registered exit hook (IPC port cleanup) before
                // dropping the task.  The hook pointer is read atomically;
                // null means no hook registered yet.
                let hook_ptr = TASK_EXIT_HOOK.load(Ordering::Acquire);
                if !hook_ptr.is_null() {
                    let hook: fn(u32) = unsafe { core::mem::transmute(hook_ptr) };
                    hook(pid);
                }

                // Save exit code before removing the task so that a concurrent
                // wait_pid() can still retrieve it after find_pid() returns None.
                log_exit(pid, exit_code);

                // Remove task from run queue.  Dropping the Task drops its
                // AddressSpace (if any), which frees all VMA backing pages and
                // the page-table root via AddressSpace::drop().
                { RUN_QUEUE.lock().remove(idx); }

                // page_table was captured above so the value is visible to
                // future tooling (debuggers, tracing).  The actual free happens
                // in AddressSpace::drop() above; we must NOT free it again here
                // (double-free).  Kernel tasks with no AddressSpace always have
                // page_table == 0, so this is always a no-op for them anyway.
                let _ = page_table;

                // Free the kernel stack allocation (order 1 = 2 × PAGE_SIZE).
                mm::buddy::free(stack_base, 1);
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
        let id  = cpu_id();
        let ctx = CURRENT_CTX[id];
        if !ctx.is_null() {
            context::cpu_switch_to(ctx, core::ptr::addr_of!(SCHEDULER_CTX[id]));
        }
    }
}

/// Block the current task until a message is sent to `port`.
///
/// The task's state is set to `Blocked` before switching to the scheduler.
/// `ipc::port::send` calls `unblock_port` to wake it.
pub fn block_on(port: u32) {
    unsafe {
        let id  = cpu_id();
        let pid = CURRENT_PID[id];
        { RUN_QUEUE.lock().block_on_port(pid, port); }
        let ctx = CURRENT_CTX[id];
        if !ctx.is_null() {
            context::cpu_switch_to(ctx, core::ptr::addr_of!(SCHEDULER_CTX[id]));
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
///
/// The exit code is stored in the task descriptor and remains readable via
/// `task_exit_code(pid)` until the task is reaped by `wait_pid()`.
pub fn exit(code: i32) -> ! {
    unsafe {
        let id  = cpu_id();
        let pid = CURRENT_PID[id];
        {
            let mut rq = RUN_QUEUE.lock();
            if let Some(idx) = rq.find_pid(pid) {
                if let Some(t) = rq.get_mut(idx) {
                    t.exit_code = code;
                }
            }
            rq.mark_zombie(pid);
        }
        let ctx = CURRENT_CTX[id];
        if !ctx.is_null() {
            context::cpu_switch_to(ctx, core::ptr::addr_of!(SCHEDULER_CTX[id]));
        }
    }
    // Unreachable: the scheduler will never switch back to a Zombie task.
    loop { core::hint::spin_loop(); }
}

/// Return the exit code of `pid` if it is a Zombie, or `None` if the task
/// does not exist or has not yet exited.
pub fn task_exit_code(pid: Pid) -> Option<i32> {
    let rq = RUN_QUEUE.lock();
    let idx = rq.find_pid(pid)?;
    let t = rq.get(idx)?;
    if t.state == TaskState::Zombie {
        Some(t.exit_code)
    } else {
        None
    }
}

/// Block the current task until `target_pid` becomes a Zombie, then return
/// its exit code and reap (remove) the task.
///
/// Returns `None` immediately if `target_pid` is not present in the run queue
/// (never existed, or was already reaped by a prior `wait_pid` call).
/// Returns `Some(exit_code)` on success.
///
/// This is a simple spin-yield loop — suitable for a cooperative scheduler.
/// A production implementation would block on a dedicated wait queue.
pub fn wait_pid(target_pid: Pid) -> Option<i32> {
    loop {
        let state = {
            let rq = RUN_QUEUE.lock();
            rq.find_pid(target_pid).map(|idx| {
                rq.get(idx).map(|t| t.state)
            })
        };

        match state {
            // PID not in run queue — either already reaped by the scheduler's
            // run() loop, or it never existed.
            //
            // Check the exit log: run() saves the exit code there before it
            // removes the task, so we can still return the correct code even
            // when the task was reaped before wait_pid() had a chance to see it.
            // Returns None (ESRCH) only if the PID was never spawned or a prior
            // wait_pid() call already consumed the log entry.
            None => return take_exit_code(target_pid),

            // Task exists and is a Zombie — reap it.
            Some(Some(TaskState::Zombie)) => {
                let code = task_exit_code(target_pid).unwrap_or(0);

                let idx = {
                    let rq = RUN_QUEUE.lock();
                    rq.find_pid(target_pid)
                };
                if let Some(idx) = idx {
                    let hook_ptr = TASK_EXIT_HOOK.load(Ordering::Acquire);
                    if !hook_ptr.is_null() {
                        let hook: fn(u32) = unsafe { core::mem::transmute(hook_ptr) };
                        hook(target_pid);
                    }
                    let stack_base = {
                        let rq = RUN_QUEUE.lock();
                        rq.get(idx).map(|t| t.kernel_stack)
                    };
                    { RUN_QUEUE.lock().remove(idx); }
                    if let Some(sb) = stack_base {
                        mm::buddy::free(sb, 1);
                    }
                }
                return Some(code);
            }

            // Task exists but is still running/ready/blocked — yield and retry.
            Some(_) => yield_now(),
        }
    }
}

/// Called from the timer ISR on every hardware tick.
///
/// Uses only atomics — safe to call from interrupt context without locks.
/// Increments the tick counter and sets PREEMPT_NEEDED so the IRQ return
/// path can force a context switch via `preempt_check()`.
pub fn timer_tick_irq() {
    TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
    PREEMPT_NEEDED.store(true, Ordering::Release);
}

/// Check and act on a pending preemption request.
///
/// Called from IRQ exit paths (after EOI) while still in interrupt context.
/// If a preemption was requested and a task is currently running, yields
/// back to the scheduler so it can pick the next runnable task.
///
/// Safe to call when no task is running (scheduler idle) — it no-ops.
pub fn preempt_check() {
    // Fast path: no preemption needed.
    if !PREEMPT_NEEDED.load(Ordering::Acquire) { return; }
    // Only preempt if a user/kernel task is on the CPU right now.
    // If CURRENT_PID == 0 we are already in the scheduler loop.
    unsafe {
        if CURRENT_PID[cpu_id()] == 0 { return; }
    }
    PREEMPT_NEEDED.store(false, Ordering::Release);
    yield_now();
}

/// Return the number of timer ticks elapsed since boot.
#[inline]
pub fn ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}

// Arch-provided: update the per-CPU kernel stack pointer used on exception
// entry from EL0/ring-3.  On x86-64 this writes TSS.rsp0; on AArch64 it
// writes TPIDR_EL1 (read by the EL0 exception entry stub to reset SP_EL1).
extern "C" {
    fn arch_set_kernel_stack(kst: u64);
}

// Arch-provided: allocate a zeroed page for a per-process page-table root
// (TTBR0_EL1 on AArch64, PML4 on x86-64).  Returns physical address or 0.
extern "C" {
    fn arch_alloc_page_table_root() -> usize;
}

// Arch-provided: switch the active user page table.
// AArch64: writes TTBR0_EL1.  x86-64: writes CR3 (no-op when root == 0).
extern "C" {
    fn arch_set_page_table(root: usize);
}

/// Spawn a user-mode task.
///
/// Allocates an 8 KiB kernel stack, a per-process page-table root, and an
/// `AddressSpace`.  Builds the arch-specific user-entry frame (`ret_to_user`
/// on AArch64, `iret_to_user` on x86-64) and enqueues the task as Ready.
/// Returns the new PID on success.
pub fn spawn_user(user_entry: usize, user_stack_top: usize, priority: i8) -> Option<Pid> {
    let stack_base = mm::buddy::alloc(1)?;
    let stack_size = mm::buddy::PAGE_SIZE * 2;
    unsafe { (stack_base as *mut u8).write_bytes(0, stack_size); }

    // Allocate a per-process page-table root (zeroed 4 KiB page).
    let page_table_root = unsafe { arch_alloc_page_table_root() };
    if page_table_root == 0 {
        mm::buddy::free(stack_base, 1);
        return None;
    }

    let pid              = alloc_pid();
    let kernel_stack_top = stack_base + stack_size;
    let ctx = CpuContext::new_user_task(user_entry, user_stack_top, kernel_stack_top);

    let mut t = Task::new_kernel(pid, 0, stack_base, stack_size, page_table_root);
    t.ctx           = ctx;
    t.priority      = priority;
    t.address_space = Some(mm::vmm::AddressSpace::new(page_table_root));

    if RUN_QUEUE.lock().enqueue(t) {
        Some(pid)
    } else {
        mm::buddy::free(stack_base, 1);
        mm::buddy::free(page_table_root, 0);
        None
    }
}

/// Run `f` with a mutable reference to the current task's `AddressSpace`.
///
/// Returns `None` if there is no current task or the task has no address space.
pub fn with_current_address_space<R>(
    f: impl FnOnce(&mut mm::vmm::AddressSpace) -> R,
) -> Option<R> {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return None; }
        let mut rq = RUN_QUEUE.lock();
        let idx = rq.find_pid(pid)?;
        let task = rq.get_mut(idx)?;
        task.address_space.as_mut().map(f)
    }
}

/// Return the current task's page-table root physical address, or 0.
pub fn current_page_table() -> usize {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return 0; }
        let rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get(idx) {
                return t.page_table;
            }
        }
        0
    }
}

/// Return the PID of the currently-running task on this CPU (0 = scheduler idle).
#[inline]
pub fn current_pid() -> u32 {
    unsafe { CURRENT_PID[cpu_id()] }
}

/// Return the reply port assigned to the current task (`u32::MAX` = not yet allocated).
pub fn current_reply_port() -> u32 {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return u32::MAX; }
        let rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get(idx) {
                return t.reply_port;
            }
        }
        u32::MAX
    }
}

/// Store `port` as the reply port for the current task.
pub fn set_current_reply_port(port: u32) {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return; }
        let mut rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get_mut(idx) {
                t.reply_port = port;
            }
        }
    }
}

/// Attempt to handle a user-mode page fault at `fault_va`.
///
/// Delegates to `mm::vmm::AddressSpace::handle_user_page_fault` on the current
/// task's address space.  Returns `true` if the fault was handled (execution
/// may resume), `false` if it should be treated as a segfault.
pub fn handle_page_fault(fault_va: usize) -> bool {
    with_current_address_space(|as_| as_.handle_user_page_fault(fault_va))
        .unwrap_or(false)
}

/// Entry point for Application Processors (APs).
///
/// Each AP calls this after arch-specific setup (stack, MMU, interrupt vectors,
/// local interrupt controller) is complete.  The AP joins the shared run queue
/// and begins scheduling tasks exactly like the BSP does in `run()`.
///
/// Never returns.
pub fn ap_entry() -> ! {
    run()
}

/// Backward-compatible alias used by the syscall dispatch table.
#[inline]
pub fn r#yield() { yield_now(); }

/// Allocate the next available PID.
///
/// PID 0 is reserved as the "no task" sentinel.  After the counter reaches
/// `u32::MAX`, it wraps to 1 so that 0 is never handed out.  PID reuse after
/// 4 billion allocations is theoretically possible but harmless in practice.
pub fn alloc_pid() -> Pid {
    let mut n = NEXT_PID.lock();
    let p = *n;
    *n = (*n).checked_add(1).unwrap_or(1); // never produce 0
    p
}
