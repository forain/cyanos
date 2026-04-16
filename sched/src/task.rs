//! Task (process/thread) descriptor — analogous to Linux `task_struct`.

use crate::context::CpuContext;
use mm::vmm::AddressSpace;

pub type Pid = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,  // Waiting on an IPC port.
    Zombie,
}

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
    /// IPC port this task is sleeping on (Some when state == Blocked).
    pub blocked_on:   Option<u32>,
    /// Per-process virtual address space (None for kernel tasks).
    pub address_space: Option<AddressSpace>,
    /// Exit status set by `exit()`.  Valid only when `state == Zombie`.
    pub exit_code:    i32,
    /// Dedicated reply port for sys_call.  Allocated at spawn; freed on exit.
    /// `u32::MAX` = not yet allocated.
    pub reply_port:   u32,
}

impl Task {
    /// Create a kernel-mode task that starts at `entry`.
    ///
    /// `stack_base` is the physical address of the stack buffer's first byte;
    /// `stack_size` is its length in bytes.
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
            address_space: None,
            exit_code:    0,
            reply_port:   u32::MAX, // allocated lazily on first sys_call
        }
    }
}
