//! Task (process/thread) descriptor — analogous to Linux `task_struct`.

pub type Pid = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,  // Waiting on IPC.
    Zombie,
}

/// Saved register state for context switching.
#[derive(Default)]
pub struct CpuContext {
    pub regs: [usize; 32], // general-purpose registers (arch-specific layout).
    pub pc: usize,
    pub sp: usize,
}

pub struct Task {
    pub pid: Pid,
    pub state: TaskState,
    pub priority: i8,        // -20..19, like Linux nice values.
    pub ctx: CpuContext,
    pub page_table: usize,   // physical address of root page table.
}

impl Task {
    pub fn new(pid: Pid, entry: usize, stack: usize, page_table: usize) -> Self {
        let mut ctx = CpuContext::default();
        ctx.pc = entry;
        ctx.sp = stack;
        Self { pid, state: TaskState::Ready, priority: 0, ctx, page_table }
    }
}
