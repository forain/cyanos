//! Cooperative scheduler — context switching, task lifecycle, IPC blocking.
//!
//! Design: single-CPU, cooperative.  Tasks run until they call `yield_now()`,
//! `block_on()`, or `exit()`.  A static idle context in `run()` is the
//! "scheduler thread" that picks the next ready task on each wake-up.
//!
//! Analogues: Linux kernel/sched/core.c (`schedule`, `switch_to`).

#![no_std]

extern crate alloc;

pub mod clone;
pub mod context;
pub mod futex;
pub mod runqueue;
pub mod signal;
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
static mut SCHEDULER_CTX: [CpuContext; MAX_CPUS] = [const { CpuContext::zeroed() }; MAX_CPUS];

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
    let entry_addr = entry as usize;
    if entry_addr == 0 {
        return None;
    }

    let pid = alloc_pid();
    let mut t = Task::new_kernel(pid, entry_addr, 0, 0, 0);

    let stack_base = mm::buddy::alloc(1)?;
    if stack_base == 0 {
        return None;
    }

    let stack_size = mm::buddy::PAGE_SIZE * 2;
    let stack_top = stack_base + stack_size;

    if stack_top <= stack_base {
        mm::buddy::free(stack_base, 1);
        return None;
    }

    t.kernel_stack = stack_base;

    unsafe {
        let ctx_ptr = (&mut t.ctx) as *mut context::CpuContext;
        let sp_ptr = (ctx_ptr as usize + core::mem::offset_of!(context::CpuContext, sp)) as *mut u64;
        core::ptr::write_volatile(sp_ptr, stack_top as u64);
    }

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
    scheduler_run_loop()
}

/// Never-returning scheduler loop that properly saves its own context.
/// This function ensures that when tasks yield back to the scheduler,
/// there is a valid return address and context to restore.
fn scheduler_run_loop() -> ! {
    // Create a local variable to hold the actual return point.
    // This ensures the scheduler has a valid context to return to.
    fn run_loop() -> ! {
        loop {
            // Run scheduler iteration
            scheduler_iteration();
        }
    }

    fn scheduler_iteration() {
        'scheduler_loop: loop {
            let maybe_idx = { RUN_QUEUE.lock().pick_next() };

            extern "C" { fn arch_serial_putc(b: u8); }
            match maybe_idx {
                Some(idx) => {
                    // Debug: Show which task we picked
                    unsafe {
                        let msg = b"[SCHED] Picked task slot ";
                        for &b in msg { arch_serial_putc(b); }
                        let mut n = idx;
                        if n == 0 { arch_serial_putc(b'0'); } else {
                            let mut buf = [0u8; 10];
                            let mut i = 10;
                            while n > 0 { i -= 1; buf[i] = b'0' + ((n % 10) as u8); n /= 10; }
                            for &c in &buf[i..] { arch_serial_putc(c); }
                        }
                        let msg2 = b"\r\n";
                        for &b in msg2 { arch_serial_putc(b); }
                    }
                }
                None => {
                    unsafe {
                        let msg = b"[SCHED] No ready task found\r\n";
                        for &b in msg { arch_serial_putc(b); }
                    }
                    continue;
                }
            }

            if let Some(idx) = maybe_idx {
                // Get task information but don't mark as Running yet
                let (ctx_ptr, pid, _kernel_stack_top, page_table) = {
                    let rq = RUN_QUEUE.lock();
                    let t = rq.get(idx).unwrap();
                    let kst = t.kernel_stack + mm::buddy::PAGE_SIZE * 2;
                    (&t.ctx as *const CpuContext, t.pid, kst, t.page_table)
                };

                unsafe {
                    let id = cpu_id();
                    CURRENT_CTX[id] = ctx_ptr as *mut CpuContext;
                    CURRENT_PID[id] = pid;

                    // Update the per-CPU kernel stack pointer used on exception entry
                    arch_set_kernel_stack(_kernel_stack_top as u64);

                    // Debug: Show we're about to context switch
                    unsafe {
                        let msg = b"[SCHED] About to call cpu_switch_to\r\n";
                        for &b in msg { arch_serial_putc(b); }
                    }

                    // Switch to userspace page table BEFORE context switch if needed
                    // NOTE: Page table switch will now happen inside cpu_switch_to_with_pt

                    // Validate userspace task context before switching
                    unsafe {
                        let task_ctx = &*ctx_ptr;
                        let msg = b"[SCHED] Validating userspace context\r\n";
                        for &b in msg { arch_serial_putc(b); }

                        // Check x30 (lr) which should point to ret_to_user
                        let ret_addr = task_ctx.gregs[11]; // x30 is gregs[11]
                        let msg2 = b"[SCHED] x30 (ret addr): 0x";
                        for &b in msg2 { arch_serial_putc(b); }
                        // Print address in hex
                        for shift in (0..16).rev() {
                            let nibble = (ret_addr >> (shift * 4)) & 0xF;
                            let ch = if nibble < 10 { b'0' + nibble as u8 } else { b'A' + (nibble - 10) as u8 };
                            arch_serial_putc(ch);
                        }
                        let msg3 = b"\r\n";
                        for &b in msg3 { arch_serial_putc(b); }

                        // Check stack pointer
                        let sp = task_ctx.sp;
                        let msg4 = b"[SCHED] Stack pointer: 0x";
                        for &b in msg4 { arch_serial_putc(b); }
                        for shift in (0..16).rev() {
                            let nibble = (sp >> (shift * 4)) & 0xF;
                            let ch = if nibble < 10 { b'0' + nibble as u8 } else { b'A' + (nibble - 10) as u8 };
                            arch_serial_putc(ch);
                        }
                        let msg5 = b"\r\n[SCHED] Starting cpu_switch_to call\r\n";
                        for &b in msg5 { arch_serial_putc(b); }
                    }

                    // Add debug immediately before cpu_switch_to call
                    unsafe {
                        let msg = b"[SCHED] About to call cpu_switch_to function directly\r\n";
                        for &b in msg { arch_serial_putc(b); }
                    }

                    // CRITICAL: Switch back to kernel page table before calling cpu_switch_to!
                    // The issue is that cpu_switch_to code is not accessible in userspace page table
                    if page_table != 0 {
                        unsafe {
                            let msg = b"[SCHED] Switching back to kernel page table before cpu_switch_to\r\n";
                            for &b in msg { arch_serial_putc(b); }
                        }
                        arch_set_page_table(0);  // Switch back to kernel page table

                        unsafe {
                            let msg = b"[SCHED] Validating pointers before function call\r\n";
                            for &b in msg { arch_serial_putc(b); }

                            // Validate scheduler context pointer
                            let sched_ctx_ptr = core::ptr::addr_of_mut!(SCHEDULER_CTX[id]);
                            if sched_ctx_ptr.is_null() {
                                let msg = b"[SCHED] ERROR: scheduler context pointer is null\r\n";
                                for &b in msg { arch_serial_putc(b); }
                                loop { core::hint::spin_loop(); }
                            }

                            // Validate task context pointer
                            if ctx_ptr.is_null() {
                                let msg = b"[SCHED] ERROR: task context pointer is null\r\n";
                                for &b in msg { arch_serial_putc(b); }
                                loop { core::hint::spin_loop(); }
                            }

                            let msg = b"[SCHED] All pointers valid, calling cpu_switch_to_with_pt\r\n";
                            for &b in msg { arch_serial_putc(b); }
                        }

                        // Try regular cpu_switch_to first to isolate the issue
                        unsafe {
                            let msg = b"[SCHED] Using regular cpu_switch_to instead\r\n";
                            for &b in msg { arch_serial_putc(b); }

                            // Check current stack pointer
                            let current_sp: u64;
                            core::arch::asm!("mov {}, sp", out(reg) current_sp);

                            let msg = b"[SCHED] Current stack pointer: 0x";
                            for &b in msg { arch_serial_putc(b); }
                            for shift in (0..16).rev() {
                                let nibble = (current_sp >> (shift * 4)) & 0xF;
                                let ch = if nibble < 10 { b'0' + nibble as u8 } else { b'A' + (nibble - 10) as u8 };
                                arch_serial_putc(ch);
                            }
                            let msg = b"\r\n";
                            for &b in msg { arch_serial_putc(b); }
                        }

                        // FIXED: Use normal context switch with page table handling in ret_to_user
                        unsafe {
                            let msg = b"[SCHED] Setting page table in task stack frame\r\n";
                            for &b in msg { arch_serial_putc(b); }

                            // Set the page table in the task's stack frame (4th word)
                            let task_ctx = &*ctx_ptr;
                            let stack_ptr = task_ctx.sp as *mut u64;
                            stack_ptr.add(3).write(page_table as u64);  // PAGE_TABLE at offset 3

                            let msg = b"[SCHED] Page table stored in frame, calling cpu_switch_to\r\n";
                            for &b in msg { arch_serial_putc(b); }

                            // Use normal context switch - ret_to_user will handle page table switch
                            context::cpu_switch_to(
                                core::ptr::addr_of_mut!(SCHEDULER_CTX[id]),
                                ctx_ptr,
                            );

                            let msg = b"[SCHED] Returned from cpu_switch_to\r\n";
                            for &b in msg { arch_serial_putc(b); }
                        }
                    } else {
                        // For kernel tasks, use normal context switch
                        context::cpu_switch_to(
                            core::ptr::addr_of_mut!(SCHEDULER_CTX[id]),
                            ctx_ptr,
                        );
                    }

                    unsafe {
                        let msg = b"[SCHED] cpu_switch_to completed\r\n";
                        for &b in msg { arch_serial_putc(b); }
                    }

                    // Debug: Show we returned from context switch
                    unsafe {
                        let msg = b"[SCHED] Returned from cpu_switch_to\r\n";
                        for &b in msg { arch_serial_putc(b); }
                    }

                    // After context switch completes, mark task as Running
                    {
                        let mut rq = RUN_QUEUE.lock();
                        if let Some(t) = rq.get_mut(idx) {
                            t.state = TaskState::Running;
                        }
                    }

                    // Task yielded back to scheduler — reset to kernel page table
                    arch_set_page_table(0);

                    CURRENT_CTX[id] = core::ptr::null_mut();
                    CURRENT_PID[id] = 0;
                }

                // Handle post-switch logic (reaping zombies, etc.)
                let zombie_info: Option<(usize, u32, usize, i32)> = {
                    let mut rq = RUN_QUEUE.lock();
                    if let Some(t) = rq.get_mut(idx) {
                        if t.state == TaskState::Zombie {
                            Some((t.kernel_stack, t.pid, t.page_table, t.exit_code))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some((stack_base, zombie_pid, page_table, exit_code)) = zombie_info {
                    // Call exit hook and cleanup
                    let hook_ptr = TASK_EXIT_HOOK.load(Ordering::Acquire);
                    if !hook_ptr.is_null() {
                        let hook: fn(u32) = unsafe { core::mem::transmute(hook_ptr) };
                        hook(zombie_pid);
                    }
                    log_exit(zombie_pid, exit_code);
                    { RUN_QUEUE.lock().remove(idx); }
                    let _ = page_table;
                    mm::buddy::free(stack_base, 1);
                }

                // Return to allow the main loop to restart
                return;
            } else {
                // No runnable task — wait for an interrupt to make one ready
                // Return to restart in main loop
                return;
            }
        }
    }

    // Call the actual run loop
    run_loop()
}


/// Voluntarily yield the rest of this task's time-slice.
///
/// The task remains Ready and will be scheduled again on the next pass.
pub fn yield_now() {
    unsafe {
        let id  = cpu_id();
        let ctx = CURRENT_CTX[id];

        if !ctx.is_null() {
            // Mark current task as Ready when it yields
            let current_pid = CURRENT_PID[id];
            if current_pid > 0 {
                let mut rq = RUN_QUEUE.lock();
                if let Some(idx) = rq.find_pid(current_pid) {
                    if let Some(task) = rq.get_mut(idx) {
                        task.state = TaskState::Ready;
                    }
                }
            }

            // Switch back to kernel page table and then to scheduler
            arch_set_page_table(0);
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
            // Switch back to kernel page table and then to scheduler
            arch_set_page_table(0);
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

        // POSIX pthread_join support: if clear_child_tid is set, atomically
        // write 0 to that address and wake any futex waiters (the joiner).
        let ctid_info: Option<(usize, usize)> = {
            let rq = RUN_QUEUE.lock();
            rq.find_pid(pid)
                .and_then(|idx| rq.get(idx))
                .and_then(|t| {
                    let ctid = t.clear_child_tid;
                    if ctid == 0 { return None; }
                    let phys = t.address_space.as_ref()?.virt_to_phys(ctid)?;
                    Some((ctid, phys))
                })
        };
        if let Some((ctid_virt, ctid_phys)) = ctid_info {
            core::ptr::write(ctid_phys as *mut u32, 0);
            futex::futex_wake(ctid_virt, u32::MAX);
        }

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
            // Switch back to kernel page table and then to scheduler
            arch_set_page_table(0);
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
            None => {
                return take_exit_code(target_pid);
            }

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
            Some(_task_state) => {
                yield_now();
            }
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

/// Spawn a user-mode task with an existing address space.
///
/// Allocates an 8 KiB kernel stack but reuses the provided address space.
/// This is used when loading ELF binaries to avoid creating a new empty
/// address space that would lose the loaded program data.
/// Returns the new PID on success.
pub fn spawn_user_with_address_space(
    user_entry: usize,
    user_stack_top: usize,
    address_space: mm::vmm::AddressSpace,
    priority: i8
) -> Option<Pid> {
    let stack_base = mm::buddy::alloc(1)?;
    let stack_size = mm::buddy::PAGE_SIZE * 2;
    unsafe { (stack_base as *mut u8).write_bytes(0, stack_size); }

    let pid              = alloc_pid();
    let kernel_stack_top = stack_base + stack_size;
    let page_table_root  = address_space.root();

    // Create proper userspace context
    let ctx = CpuContext::new_user_task(user_entry, user_stack_top, kernel_stack_top);

    let mut t = Task::new_kernel(pid, 0, stack_base, stack_size, page_table_root);
    t.ctx           = ctx;
    t.priority      = priority;
    t.address_space = Some(address_space);

    if RUN_QUEUE.lock().enqueue(t) {
        Some(pid)
    } else {
        mm::buddy::free(stack_base, 1);
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

// ── Signal API ───────────────────────────────────────────────────────────────

/// Deliver pending signals to the current task at a return-to-user-space path.
///
/// `frame_ptr` — address of the `UserFrame` saved on the kernel stack by the
/// EL0→EL1 exception entry stub (AArch64 only; 0 on x86-64).
///
/// This function is called with `#[no_mangle]` from `signal::check_and_deliver_signals`
/// so that the AArch64 exception assembly can branch to it directly by symbol.
pub fn check_and_deliver_signals(frame_ptr: usize) {
    signal::check_and_deliver_signals(frame_ptr);
}

/// Restore the user context from a saved `rt_sigframe` (called by `sys_rt_sigreturn`).
pub fn restore_signal_frame(frame_ptr: usize) {
    signal::restore_signal_frame(frame_ptr);
}

// ── Signal action/mask API ────────────────────────────────────────────────────

/// Install or query a signal action for the current task.
///
/// `act_ptr`    — pointer to a new `SigAction` (0 = query only).
/// `oldact_ptr` — pointer to write the previous `SigAction` (0 = discard).
pub fn sys_sigaction(signum: u32, act_ptr: usize, oldact_ptr: usize) -> isize {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return -1; }
        let mut rq = RUN_QUEUE.lock();
        let idx = match rq.find_pid(pid) { Some(i) => i, None => return -3 };
        let task = match rq.get_mut(idx) { Some(t) => t, None => return -3 };

        if oldact_ptr != 0 {
            core::ptr::write(oldact_ptr as *mut task::SigAction,
                task.signal_actions[signum as usize]);
        }
        if act_ptr != 0 {
            let new_act = core::ptr::read(act_ptr as *const task::SigAction);
            task.signal_actions[signum as usize] = new_act;
        }
    }
    0
}

/// Set or query the signal mask for the current task.
///
/// `how`: 0 = SIG_BLOCK, 1 = SIG_UNBLOCK, 2 = SIG_SETMASK.
pub fn sys_sigprocmask(how: usize, set_ptr: usize, oldset_ptr: usize) -> isize {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return -1; }
        let mut rq = RUN_QUEUE.lock();
        let idx = match rq.find_pid(pid) { Some(i) => i, None => return -3 };
        let task = match rq.get_mut(idx) { Some(t) => t, None => return -3 };

        if oldset_ptr != 0 {
            core::ptr::write(oldset_ptr as *mut u64, task.signal_mask);
        }
        if set_ptr != 0 {
            let set = core::ptr::read(set_ptr as *const u64);
            task.signal_mask = match how {
                0 => task.signal_mask | set,   // SIG_BLOCK
                1 => task.signal_mask & !set,  // SIG_UNBLOCK
                2 => set,                      // SIG_SETMASK
                _ => return -22,               // EINVAL
            };
        }
    }
    0
}

/// Set the pending signal bit for `target_pid`.
///
/// Returns 0 on success, -3 (ESRCH) if the task does not exist.
pub fn deliver_signal(target_pid: task::Pid, sig: u32) -> isize {
    if sig == 0 { return 0; } // signal 0 = existence check
    let mut rq = RUN_QUEUE.lock();
    let idx = match rq.find_pid(target_pid) { Some(i) => i, None => return -3 };
    if let Some(t) = rq.get_mut(idx) {
        if sig < 64 { t.signal_pending |= 1u64 << sig; }
        // If the task is blocked, unblock it so it can handle the signal.
        if t.state == task::TaskState::Blocked {
            t.state = task::TaskState::Ready;
        }
    }
    0
}

/// Return the signal_pending bitmask for the current task.
pub fn pending_signals() -> u64 {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return 0; }
        let rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get(idx) { return t.signal_pending; }
        }
        0
    }
}

/// Atomically replace the current task's signal mask.  Returns the old mask.
pub fn replace_signal_mask(new_mask: u64) -> u64 {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return 0; }
        let mut rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get_mut(idx) {
                let old = t.signal_mask;
                t.signal_mask = new_mask;
                return old;
            }
        }
        0
    }
}

/// Clear a single pending signal from the current task (used by rt_sigtimedwait).
pub fn clear_pending_signal(signo: u32) {
    if signo == 0 || signo > 64 { return; }
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return; }
        let mut rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get_mut(idx) {
                t.signal_pending &= !(1u64 << (signo - 1));
            }
        }
    }
}

/// Return the parent PID of the current task.
pub fn current_ppid() -> task::Pid {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return 0; }
        let rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get(idx) { return t.ppid; }
        }
        0
    }
}

/// Copy the current task's working directory into `buf[..size]`.
/// Returns the length written (not including NUL), or -1 on error.
pub fn current_cwd(buf: *mut u8, size: usize) -> isize {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return -1; }
        let rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get(idx) {
                let len = t.cwd_len.min(size.saturating_sub(1));
                core::ptr::copy_nonoverlapping(t.cwd.as_ptr(), buf, len);
                *buf.add(len) = 0;
                return len as isize;
            }
        }
        -1
    }
}

/// Set the current task's working directory.  `path` must be absolute.
pub fn set_cwd(path: &[u8]) -> bool {
    if path.is_empty() || path.len() > 255 { return false; }
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return false; }
        let mut rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get_mut(idx) {
                let len = path.len().min(255);
                t.cwd[..len].copy_from_slice(&path[..len]);
                t.cwd[len] = 0;
                t.cwd_len  = len;
                return true;
            }
        }
        false
    }
}

/// Return the current task's umask; optionally set a new one.
/// Pass `new_mask = u32::MAX` to query without modifying.
pub fn umask(new_mask: u32) -> u32 {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return 0o022; }
        let mut rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get_mut(idx) {
                let old = t.umask;
                if new_mask != u32::MAX { t.umask = new_mask & 0o777; }
                return old;
            }
        }
        0o022
    }
}

/// Return the process group ID of the current task.
pub fn current_pgid() -> task::Pid {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return 0; }
        let rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get(idx) { return t.pgid; }
        }
        0
    }
}

/// Set the process group ID of task `pid` to `pgid`.
pub fn set_pgid(pid: task::Pid, pgid: task::Pid) -> bool {
    let mut rq = RUN_QUEUE.lock();
    if let Some(idx) = rq.find_pid(pid) {
        if let Some(t) = rq.get_mut(idx) { t.pgid = pgid; return true; }
    }
    false
}

/// Create a new session for the current task.  Returns the new SID.
pub fn setsid() -> task::Pid {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return 0; }
        let mut rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get_mut(idx) {
                t.sid  = pid;
                t.pgid = pid;
                return pid;
            }
        }
        0
    }
}

/// Return the current task's heap end (program break), or 0 if not set.
pub fn heap_end() -> isize {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return 0; }
        let rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get(idx) { return t.heap_end as isize; }
        }
        0
    }
}

/// Return the session ID of the current task.
pub fn current_sid() -> task::Pid {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return 0; }
        let rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get(idx) { return t.sid; }
        }
        0
    }
}

// ── Thread / futex API ────────────────────────────────────────────────────────

/// Record the `clear_child_tid` address for the current task (for `set_tid_address`).
pub fn set_clear_child_tid(tidptr: usize) {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return; }
        let mut rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get_mut(idx) { t.clear_child_tid = tidptr; }
        }
    }
}

// ── TLS register helpers (x86-64 FS.base) ────────────────────────────────────

/// Store the new FS.base in the current task's CpuContext so it is
/// restored on the next context switch back to this task.
pub fn set_fs_base(base: u64) {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return; }
        let mut rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get_mut(idx) {
                #[cfg(not(target_arch = "aarch64"))]
                { t.ctx.fs_base = base; }
                #[cfg(target_arch = "aarch64")]
                { t.ctx.tpidr_el0 = base; }
                t.tls_base = base;
            }
        }
    }
}

/// Read the FS.base saved in the current task's CpuContext.
pub fn get_fs_base() -> u64 {
    unsafe {
        let pid = CURRENT_PID[cpu_id()];
        if pid == 0 { return 0; }
        let rq = RUN_QUEUE.lock();
        if let Some(idx) = rq.find_pid(pid) {
            if let Some(t) = rq.get(idx) {
                return t.tls_base;
            }
        }
        0
    }
}

// ── Arch-provided: jump to user space at a new entry point ───────────────────
extern "C" {
    fn arch_execve_return(entry: usize, user_sp: usize) -> !;
}

/// Replace the current task's address space with `new_as` and transfer
/// execution to `entry` / `user_sp` in the new address space.
///
/// 1. Switches the hardware page table to `pt_root` (the new AS's root).
/// 2. Stores `new_as` in the task, dropping (and freeing) the old AS.
/// 3. Calls `arch_execve_return(entry, user_sp)` — never returns.
///
/// # Safety
///
/// `new_as` must be fully constructed (all PT_LOAD segments mapped, user stack
/// mapped) before calling this function.  `pt_root` must be the page-table
/// root stored inside `new_as`.
pub fn replace_address_space(
    new_as:     mm::vmm::AddressSpace,
    pt_root:    usize,
    heap_start: usize,
    entry:      usize,
    user_sp:    usize,
) -> ! {
    unsafe {
        let id  = cpu_id();
        let pid = CURRENT_PID[id];

        // Switch the hardware page table BEFORE dropping the old one so that
        // the CPU is never executing with a freed page-table root.
        arch_set_page_table(pt_root);

        // Replace the address space.  Assigning Some(new_as) drops the old
        // AddressSpace, which unmaps all VMAs, frees physical pages, frees
        // the old PT root, and issues a TLB shootdown.
        {
            let mut rq = RUN_QUEUE.lock();
            if let Some(idx) = rq.find_pid(pid) {
                if let Some(t) = rq.get_mut(idx) {
                    t.address_space = Some(new_as);
                    t.page_table    = pt_root;
                    t.heap_start    = heap_start;
                    t.heap_end      = heap_start;
                }
            }
        }

        // Jump to user space.  Does not return.
        arch_execve_return(entry, user_sp);
    }
}

/// Fork the currently-running task.
///
/// Thin public wrapper around [`clone::fork_current`].  The `frame_ptr`
/// argument is the kernel-stack address of the `UserFrame` saved by the
/// AArch64 EL0 exception stub (0 on x86-64).
///
/// Returns the child PID to the parent on success, or a negative errno.
pub fn fork_current(frame_ptr: usize) -> isize {
    clone::fork_current(frame_ptr)
}

/// Spawn a new thread sharing the current process's virtual address space.
///
/// Thin public wrapper around [`clone::clone_thread`].
pub fn clone_thread(
    flags:       usize,
    child_stack: usize,
    tls:         usize,
    ctid:        usize,
    frame_ptr:   usize,
) -> isize {
    clone::clone_thread(flags, child_stack, tls, ctid, frame_ptr)
}

/// Block the current task on `uaddr` (FUTEX_WAIT path).
pub fn futex_wait(uaddr: usize, timeout_ptr: usize) -> isize {
    futex::futex_wait(uaddr, timeout_ptr)
}

/// Wake up to `n` tasks blocked on `uaddr` (FUTEX_WAKE path).
pub fn futex_wake(uaddr: usize, n: u32) -> u32 {
    futex::futex_wake(uaddr, n)
}

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

// Helper functions for debug output
fn serial_print(s: &str) {
    for b in s.bytes() {
        unsafe {
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
        if b == b'\n' {
            unsafe {
                #[cfg(target_arch = "x86_64")]
                {
                    use core::arch::asm;
                    loop {
                        let mut status: u8;
                        asm!("in al, dx", out("al") status, in("dx") 0x3FDu16,
                             options(nomem, nostack));
                        if status & 0x20 != 0 { break; }
                    }
                    asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'\r',
                         options(nomem, nostack));
                }
                #[cfg(target_arch = "aarch64")]
                {
                    #[cfg(not(feature = "rpi5"))]
                    let base = 0x09000000usize;
                    #[cfg(feature = "rpi5")]
                    let base = 0x107D_0010_00usize;
                    let fr = (base + 0x18) as *const u32;
                    while fr.read_volatile() & (1 << 5) != 0 {}
                    let dr = base as *mut u32;
                    dr.write_volatile(b'\r' as u32);
                }
            }
        }
    }
}

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
        unsafe {
            #[cfg(target_arch = "x86_64")]
            {
                use core::arch::asm;
                loop {
                    let mut status: u8;
                    asm!("in al, dx", out("al") status, in("dx") 0x3FDu16,
                         options(nomem, nostack));
                    if status & 0x20 != 0 { break; }
                }
                asm!("out dx, al", in("dx") 0x3F8u16, in("al") c,
                     options(nomem, nostack));
            }
            #[cfg(target_arch = "aarch64")]
            {
                #[cfg(not(feature = "rpi5"))]
                let base = 0x09000000usize;
                #[cfg(feature = "rpi5")]
                let base = 0x107D_0010_00usize;
                let fr = (base + 0x18) as *const u32;
                while fr.read_volatile() & (1 << 5) != 0 {}
                let dr = base as *mut u32;
                dr.write_volatile(c as u32);
            }
        }
    }
}

fn print_pid(pid: u32) {
    if pid == 0 {
        serial_print("0");
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 10;
    let mut n = pid;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + ((n % 10) as u8);
        n /= 10;
    }
    let s = unsafe { core::str::from_utf8_unchecked(&buf[i..]) };
    serial_print(s);
}
