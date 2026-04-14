//! Run queue — fixed-size array of task slots with round-robin selection.
//!
//! Future: replace with a red-black tree keyed on virtual runtime (à la CFS).

use super::task::{Pid, Task, TaskState};

pub const MAX_TASKS: usize = 256;

pub struct RunQueue {
    pub tasks: [Option<Task>; MAX_TASKS],
    len:       usize,
    cursor:    usize,
}

impl RunQueue {
    pub const fn new() -> Self {
        Self { tasks: [const { None }; MAX_TASKS], len: 0, cursor: 0 }
    }

    /// Insert a task into the first free slot. Returns false if the queue is full.
    pub fn enqueue(&mut self, task: Task) -> bool {
        for slot in &mut self.tasks {
            if slot.is_none() {
                *slot = Some(task);
                self.len += 1;
                return true;
            }
        }
        false
    }

    /// Pick the next Ready task (round-robin). Marks it Running.
    /// Returns the slot index so the caller can track which task is active.
    pub fn pick_next(&mut self) -> Option<usize> {
        if self.len == 0 { return None; }
        for _ in 0..MAX_TASKS {
            self.cursor = (self.cursor + 1) % MAX_TASKS;
            let c = self.cursor;
            if let Some(task) = &mut self.tasks[c] {
                if task.state == TaskState::Ready {
                    task.state = TaskState::Running;
                    return Some(c);
                }
            }
        }
        None
    }

    pub fn get_mut(&mut self, idx: usize) -> Option<&mut Task> {
        self.tasks[idx].as_mut()
    }

    /// Block the task with `pid`, recording the port it is waiting on.
    pub fn block_on_port(&mut self, pid: Pid, port: u32) {
        for slot in &mut self.tasks {
            if let Some(task) = slot {
                if task.pid == pid {
                    task.state      = TaskState::Blocked;
                    task.blocked_on = Some(port);
                    return;
                }
            }
        }
    }

    /// Wake all tasks blocked on `port`.
    pub fn unblock_port(&mut self, port: u32) {
        for slot in &mut self.tasks {
            if let Some(task) = slot {
                if task.blocked_on == Some(port) && task.state == TaskState::Blocked {
                    task.state      = TaskState::Ready;
                    task.blocked_on = None;
                }
            }
        }
    }

    /// Mark a task as Zombie (terminal; will not be scheduled again).
    pub fn mark_zombie(&mut self, pid: Pid) {
        for slot in &mut self.tasks {
            if let Some(task) = slot {
                if task.pid == pid {
                    task.state = TaskState::Zombie;
                    return;
                }
            }
        }
    }
}
