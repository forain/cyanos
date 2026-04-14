//! IPC Port — named endpoint for message passing between tasks.
//!
//! Analogous to a Unix socket bound to an address, or an L4 endpoint cap.
//! `send` is non-blocking (returns false if the queue is full).
//! `recv` is non-blocking (returns None if empty); callers that need to block
//! should call `sched::block_on(port)` when recv returns None.

use spin::Mutex;
use super::message::Message;

pub type Port = u32;

const MAX_PORTS:  usize = 1024;
const QUEUE_DEPTH: usize = 16;

struct PortEntry {
    owner_pid: u32,
    queue:     [Option<Message>; QUEUE_DEPTH],
    head:      usize,
    tail:      usize,
}

impl PortEntry {
    const fn empty() -> Self {
        Self {
            owner_pid: 0,
            queue: [const { None }; QUEUE_DEPTH],
            head: 0,
            tail: 0,
        }
    }

    fn enqueue(&mut self, msg: Message) -> bool {
        let next = (self.tail + 1) % QUEUE_DEPTH;
        if next == self.head { return false; } // full
        self.queue[self.tail] = Some(msg);
        self.tail = next;
        true
    }

    fn dequeue(&mut self) -> Option<Message> {
        if self.head == self.tail { return None; }
        let msg = self.queue[self.head].take();
        self.head = (self.head + 1) % QUEUE_DEPTH;
        msg
    }
}

static PORTS: Mutex<[Option<PortEntry>; MAX_PORTS]> =
    Mutex::new([const { None }; MAX_PORTS]);

pub fn init() {}

/// Allocate a new port owned by `pid`.  Returns the port number.
pub fn create(pid: u32) -> Option<Port> {
    let mut ports = PORTS.lock();
    for (i, slot) in ports.iter_mut().enumerate() {
        if slot.is_none() {
            let mut e = PortEntry::empty();
            e.owner_pid = pid;
            *slot = Some(e);
            return Some(i as Port);
        }
    }
    None
}

/// Enqueue `msg` on `port`.  Returns false if the port is full or invalid.
///
/// Wakes any task blocked on this port via `sched::unblock_port`.
pub fn send(port: Port, msg: Message) -> bool {
    // Release the port lock before waking the blocked task to avoid
    // lock-order issues (unblock_port acquires the run-queue lock).
    let enqueued = {
        let mut ports = PORTS.lock();
        match ports.get_mut(port as usize).and_then(|s| s.as_mut()) {
            Some(entry) => entry.enqueue(msg),
            None        => false,
        }
    };
    if enqueued {
        sched::unblock_port(port);
    }
    enqueued
}

/// Dequeue one message from `port`.  Returns None if the queue is empty.
pub fn recv(port: Port) -> Option<Message> {
    let mut ports = PORTS.lock();
    ports.get_mut(port as usize)?.as_mut()?.dequeue()
}
