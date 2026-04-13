//! Run queue — ordered collection of ready tasks.
//!
//! Current implementation: simple fixed-size ring buffer.
//! Future: red-black tree keyed on virtual runtime (à la CFS).

use super::task::{Task, TaskState};

const MAX_TASKS: usize = 256;

pub struct RunQueue {
    tasks: [Option<Task>; MAX_TASKS],
    len: usize,
    cursor: usize,
}

impl RunQueue {
    pub const fn new() -> Self {
        Self { tasks: [const { None }; MAX_TASKS], len: 0, cursor: 0 }
    }

    pub fn enqueue(&mut self, task: Task) -> bool {
        for slot in &mut self.tasks {
            if slot.is_none() {
                *slot = Some(task);
                self.len += 1;
                return true;
            }
        }
        false // queue full
    }

    /// Pick the next ready task using round-robin.
    pub fn pick_next(&mut self) -> Option<&mut Task> {
        if self.len == 0 { return None; }
        for _ in 0..MAX_TASKS {
            self.cursor = (self.cursor + 1) % MAX_TASKS;
            if let Some(task) = &mut self.tasks[self.cursor] {
                if task.state == TaskState::Ready {
                    task.state = TaskState::Running;
                    return Some(task);
                }
            }
        }
        None
    }
}
