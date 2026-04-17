//! Task (process/thread) descriptor — analogous to Linux `task_struct`.

use crate::context::CpuContext;
use mm::vmm::AddressSpace;

pub type Pid = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,  // Waiting on an IPC port or futex.
    Zombie,
}

/// Per-signal disposition, matching the POSIX `struct sigaction` layout.
#[derive(Clone, Copy)]
pub struct SigAction {
    /// Handler address: 0 = SIG_DFL, 1 = SIG_IGN, else a user-space fn ptr.
    pub handler:  usize,
    pub flags:    u32,
    /// Signal mask to apply during handler execution.
    pub mask:     u64,
    /// `sa_restorer` — user-space trampoline that calls `sys_rt_sigreturn`.
    /// 0 = use the kernel's built-in trampoline page.
    pub restorer: usize,
}

pub const DEFAULT_SIGACTION: SigAction =
    SigAction { handler: 0, flags: 0, mask: 0, restorer: 0 };

pub struct Task {
    pub pid:          Pid,
    pub state:        TaskState,
    pub priority:     i8,
    /// Saved CPU register state.
    pub ctx:          CpuContext,
    /// Root page table physical address (0 = use kernel tables).
    pub page_table:   usize,
    /// Physical address of the bottom of this task's kernel stack allocation.
    pub kernel_stack: usize,
    /// IPC port this task is sleeping on (Some when state == Blocked on IPC).
    pub blocked_on:   Option<u32>,
    /// Futex user-space address this task is waiting on (0 = none).
    pub blocked_futex: usize,
    /// Per-process virtual address space (None for kernel tasks).
    pub address_space: Option<AddressSpace>,
    /// Exit status set by `exit()`.  Valid only when `state == Zombie`.
    pub exit_code:    i32,
    /// Dedicated reply port for sys_call.  Allocated at spawn; freed on exit.
    /// `u32::MAX` = not yet allocated.
    pub reply_port:   u32,

    // ── POSIX process identity ────────────────────────────────────────────────
    pub ppid: Pid,   // parent PID
    pub tgid: Pid,   // thread group leader PID (== pid for single-threaded tasks)
    pub pgid: Pid,   // process group ID
    pub sid:  Pid,   // session ID

    // ── POSIX credentials ─────────────────────────────────────────────────────
    pub uid:  u32,
    pub gid:  u32,
    pub euid: u32,
    pub egid: u32,

    // ── Signal state ──────────────────────────────────────────────────────────
    /// Bitmask of pending signals (bit N = signal N+1 is pending).
    pub signal_pending: u64,
    /// Bitmask of blocked (masked) signals.
    pub signal_mask:    u64,
    /// Per-signal disposition table (64 signals).
    pub signal_actions: [SigAction; 64],

    // ── Thread state ──────────────────────────────────────────────────────────
    /// User-space address of the thread's TID word (for `set_tid_address`).
    /// Written to 0 and futex-woken on thread exit so `pthread_join` works.
    pub clear_child_tid: usize,

    // ── Heap bookmarks (for sys_brk) ─────────────────────────────────────────
    pub heap_start: usize,
    pub heap_end:   usize,

    // ── Architecture-specific TLS register ───────────────────────────────────
    /// x86-64: FS.base (thread-local storage pointer), saved/restored on switch.
    /// AArch64: TPIDR_EL0, saved/restored on switch.
    pub tls_base: u64,

    // ── Filesystem state ──────────────────────────────────────────────────────
    /// Current working directory (null-terminated path, max 255 bytes + NUL).
    pub cwd:     [u8; 256],
    pub cwd_len: usize,
    /// File-creation mask (POSIX umask).
    pub umask:   u32,
}

impl Task {
    /// Create a kernel-mode task that starts at `entry`.
    pub fn new_kernel(
        pid:        Pid,
        entry:      usize,
        stack_base: usize,
        stack_size: usize,
        page_table: usize,
    ) -> Self {
        let stack_top = stack_base + stack_size;
        Self {
            pid,
            state:        TaskState::Ready,
            priority:     0,
            ctx:          CpuContext::new_task(entry, stack_top),
            page_table,
            kernel_stack: stack_base,
            blocked_on:   None,
            blocked_futex: 0,
            address_space: None,
            exit_code:    0,
            reply_port:   u32::MAX,
            ppid:         0,
            tgid:         pid,
            pgid:         pid,
            sid:          pid,
            uid:          0,
            gid:          0,
            euid:         0,
            egid:         0,
            signal_pending: 0,
            signal_mask:    0,
            signal_actions: [DEFAULT_SIGACTION; 64],
            clear_child_tid: 0,
            heap_start:   0,
            heap_end:     0,
            tls_base:     0,
            cwd:          { let mut a = [0u8; 256]; a[0] = b'/'; a },
            cwd_len:      1,
            umask:        0o022,
        }
    }
}
