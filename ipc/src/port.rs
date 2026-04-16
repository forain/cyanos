//! IPC Port — named endpoint for message passing between tasks.
//!
//! Analogous to a Unix socket bound to an address, or an L4 endpoint cap.
//! `send` is non-blocking; on success it returns `Ok(())`, on failure it
//! returns a `SendError` that distinguishes queue-full from invalid port.
//! `recv` is non-blocking (returns None if empty); callers that need to block
//! should call `sched::block_on(port)` when recv returns None.
//!
//! # Hard limits
//!
//! | Constant      | Value | Notes                                          |
//! |---------------|-------|------------------------------------------------|
//! | `MAX_PORTS`   | 1024  | Maximum number of simultaneously open ports.  |
//! | `QUEUE_DEPTH` | 16    | Per-port message queue capacity.               |
//!
//! If `MAX_PORTS` ports are open, `create()` returns `None`.
//! If a port's queue is full, `send()` returns `Err(SendError::QueueFull)`.
//! Callers should check the return value and apply backpressure or yield.

use spin::Mutex;
use super::message::Message;

pub type Port = u32;

/// Maximum number of simultaneously open IPC ports.
pub const MAX_PORTS:   usize = 1024;
/// Per-port message queue capacity (number of messages before send() blocks).
pub const QUEUE_DEPTH: usize = 16;

/// Error returned by [`send`] when the message cannot be delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The port number is not currently open (invalid or already closed).
    PortNotFound,
    /// The port's message queue is full (`QUEUE_DEPTH` messages pending).
    /// The caller should yield and retry, or apply backpressure upstream.
    QueueFull,
}

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

/// Close all ports owned by `pid` and drain their queues.
///
/// Called from the zombie-reaping path so that port IDs are reclaimed
/// after a task exits.  Safe to call with no ports owned (no-op).
///
/// After closing each port, wakes any task that is blocked inside
/// `sys_recv` on that port ID so it can re-check and return EBADF
/// instead of sleeping forever.
pub fn release_by_owner(pid: u32) {
    // Collect the port IDs being closed so we can release the PORTS lock
    // before calling sched::unblock_port (which acquires the run-queue lock).
    // Holding both locks simultaneously would risk a lock-order deadlock.
    let mut closed: [Option<Port>; 32] = [None; 32];
    let mut n_closed = 0usize;

    {
        let mut ports = PORTS.lock();
        for (i, slot) in ports.iter_mut().enumerate() {
            if let Some(entry) = slot {
                if entry.owner_pid == pid {
                    *slot = None;
                    if n_closed < closed.len() {
                        closed[n_closed] = Some(i as Port);
                        n_closed += 1;
                    }
                }
            }
        }
    } // PORTS lock released here

    // Wake any tasks blocked on the now-closed ports.
    for i in 0..n_closed {
        if let Some(port) = closed[i] {
            sched::unblock_port(port);
        }
    }
}

/// Close `port` and free its slot for reuse.
///
/// If the port does not exist this is a no-op (idempotent).
/// Does **not** wake tasks blocked on the port; use `release_by_owner`
/// if waiting receivers must also be unblocked.
pub fn close(port: Port) {
    if let Some(slot) = PORTS.lock().get_mut(port as usize) {
        *slot = None;
    }
}

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

/// Enqueue `msg` on `port`.
///
/// Returns `Ok(())` on success.
/// Returns `Err(SendError::PortNotFound)` if the port is not open.
/// Returns `Err(SendError::QueueFull)` if the port's queue is at capacity
/// (`QUEUE_DEPTH` messages).  The caller should yield and retry, or drop
/// the message and signal backpressure upstream.
///
/// On success, wakes any task blocked on this port via `sched::unblock_port`.
pub fn send(port: Port, msg: Message) -> Result<(), SendError> {
    // Release the port lock before waking the blocked task to avoid
    // lock-order issues (unblock_port acquires the run-queue lock).
    let result: Result<(), SendError> = {
        let mut ports = PORTS.lock();
        match ports.get_mut(port as usize).and_then(|s| s.as_mut()) {
            None        => Err(SendError::PortNotFound),
            Some(entry) => {
                if entry.enqueue(msg) { Ok(()) }
                else                  { Err(SendError::QueueFull) }
            }
        }
    };
    if result.is_ok() {
        sched::unblock_port(port);
    }
    result
}

/// Return the current queue depth and maximum capacity for `port`.
///
/// Returns `Some((depth, capacity))` where `depth` is the number of messages
/// currently queued and `capacity` is `QUEUE_DEPTH`.
/// Returns `None` if the port does not exist.
///
/// Use this to detect backpressure before sending: if `depth == capacity - 1`,
/// the next `send` may return `Err(SendError::QueueFull)`.
pub fn port_stats(port: Port) -> Option<(usize, usize)> {
    let ports = PORTS.lock();
    let entry = ports.get(port as usize)?.as_ref()?;
    // Circular buffer depth: (tail - head + QUEUE_DEPTH) % QUEUE_DEPTH.
    let depth = (entry.tail + QUEUE_DEPTH - entry.head) % QUEUE_DEPTH;
    Some((depth, QUEUE_DEPTH - 1)) // capacity = QUEUE_DEPTH - 1 (one slot reserved)
}

/// Dequeue one message from `port`.  Returns None if the queue is empty.
pub fn recv(port: Port) -> Option<Message> {
    let mut ports = PORTS.lock();
    ports.get_mut(port as usize)?.as_mut()?.dequeue()
}

/// Like `recv`, but additionally enforces that `caller_pid` owns the port.
///
/// Returns `None` both when the queue is empty and when the caller is not
/// the port owner.  Callers should distinguish ownership failure by calling
/// `is_owner` first if they need an explicit error.
pub fn recv_as(port: Port, caller_pid: u32) -> Option<Message> {
    let mut ports = PORTS.lock();
    let entry = ports.get_mut(port as usize)?.as_mut()?;
    if entry.owner_pid != caller_pid { return None; }
    entry.dequeue()
}

/// Returns true if `pid` is the owner of `port`.
pub fn is_owner(port: Port, pid: u32) -> bool {
    let ports = PORTS.lock();
    ports.get(port as usize)
        .and_then(|s| s.as_ref())
        .map(|e| e.owner_pid == pid)
        .unwrap_or(false)
}
