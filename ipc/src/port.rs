//! IPC Port — named endpoint analogous to a socket bound to an address.

use spin::Mutex;
use super::message::Message;

pub type Port = u32;

const MAX_PORTS: usize = 1024;
const QUEUE_DEPTH: usize = 16;

struct PortEntry {
    owner_pid: u32,
    queue: [Option<Message>; QUEUE_DEPTH],
    head: usize,
    tail: usize,
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

/// Register a new port owned by `pid`. Returns the allocated port number.
pub fn create(pid: u32) -> Option<Port> {
    let mut ports = PORTS.lock();
    for (i, slot) in ports.iter_mut().enumerate() {
        if slot.is_none() {
            let mut entry = PortEntry::empty();
            entry.owner_pid = pid;
            *slot = Some(entry);
            return Some(i as Port);
        }
    }
    None
}

/// Send a message to `port`. Non-blocking — returns false if queue is full.
pub fn send(port: Port, msg: Message) -> bool {
    let mut ports = PORTS.lock();
    if let Some(Some(entry)) = ports.get_mut(port as usize) {
        return entry.enqueue(msg);
    }
    false
}

/// Receive a message from `port`. Returns None if queue is empty.
pub fn recv(port: Port) -> Option<Message> {
    let mut ports = PORTS.lock();
    if let Some(Some(entry)) = ports.get_mut(port as usize) {
        return entry.dequeue();
    }
    None
}
