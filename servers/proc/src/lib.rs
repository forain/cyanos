//! Process server — owns the global PID→port mapping, wait4 zombie list,
//! process credentials policy, process groups, and sessions.
//!
//! # Message encoding
//!
//! Arguments are packed into `Message.data` as little-endian `u64` words:
//!   data[0..8]  = arg0, data[8..16] = arg1, data[16..24] = arg2
//!
//! | Tag                | arg0     | arg1     | arg2   | Reply arg0 |
//! |--------------------|----------|----------|--------|------------|
//! | PROC_REGISTER      | pid      | ppid     | pgid   | 0 = ok     |
//! | PROC_NOTIFY_EXIT   | pid      | exit_code| 0      | 0 = ok     |
//! | PROC_WAIT4         | pid / -1 | 0        | 0      | exit_code  |
//! | PROC_SETPGID       | pid      | pgid     | 0      | 0 = ok     |
//! | PROC_SETSID        | pid      | 0        | 0      | new_sid    |
//! | PROC_GETSID        | pid      | 0        | 0      | sid        |
//! | PROC_SETCRED       | uid      | gid      | euid   | 0 / -EPERM |

#![no_std]

use ipc::{Message, port};
use spin::Mutex;

// ── Protocol tag constants ────────────────────────────────────────────────────

pub const PROC_REGISTER:    u64 = 0x01;
pub const PROC_NOTIFY_EXIT: u64 = 0x02;
pub const PROC_WAIT4:       u64 = 0x03;
pub const PROC_SETPGID:     u64 = 0x04;
pub const PROC_SETSID:      u64 = 0x05;
pub const PROC_GETSID:      u64 = 0x06;
pub const PROC_SETCRED:     u64 = 0x07;

// ── Message helpers ───────────────────────────────────────────────────────────

/// Read the N-th `u64` argument from `msg.data`.
#[inline]
fn arg(msg: &Message, n: usize) -> u64 {
    let off = n * 8;
    u64::from_le_bytes(msg.data[off..off + 8].try_into().unwrap_or([0u8; 8]))
}

fn make_reply(v: i64) -> Message {
    let mut m = Message::empty();
    m.data[0..8].copy_from_slice(&(v as u64).to_le_bytes());
    m
}

fn ok_reply()          -> Message { make_reply(0) }
fn err_reply(e: i32)   -> Message { make_reply(e as i64) }
fn val_reply(v: u64)   -> Message { make_reply(v as i64) }

// ── Process table ─────────────────────────────────────────────────────────────

const MAX_PROCS: usize = 256;

#[derive(Clone, Copy)]
#[allow(dead_code)]
struct ProcEntry {
    pid:   u32,
    ppid:  u32,
    pgid:  u32,
    sid:   u32,
    uid:   u32,
    gid:   u32,
    euid:  u32,
    egid:  u32,
    port:  u32,
    state: ProcState,
}

#[derive(Clone, Copy, PartialEq)]
enum ProcState {
    Empty,
    Alive,
    Zombie { exit_code: i32 },
}

impl ProcEntry {
    const fn empty() -> Self {
        Self {
            pid: 0, ppid: 0, pgid: 0, sid: 0,
            uid: 0, gid: 0, euid: 0, egid: 0,
            port: 0,
            state: ProcState::Empty,
        }
    }
}

static TABLE: Mutex<[ProcEntry; MAX_PROCS]> =
    Mutex::new([const { ProcEntry::empty() }; MAX_PROCS]);

// ── Pending wait4 requests ────────────────────────────────────────────────────

const MAX_WAITERS: usize = 64;

#[derive(Clone, Copy)]
struct WaitReq {
    target_pid:  u32,  // 0 = wait for any child
    reply_port:  u32,
}

impl WaitReq {}

static WAIT_QUEUE: Mutex<[Option<WaitReq>; MAX_WAITERS]> =
    Mutex::new([const { None }; MAX_WAITERS]);

// ── Server well-known port ────────────────────────────────────────────────────

static SERVER_PORT: Mutex<u32> = Mutex::new(u32::MAX);

/// Initialise the process server and return its IPC port ID.
pub fn init(owner_pid: u32) -> Option<u32> {
    let port_id = port::create(owner_pid)?;
    *SERVER_PORT.lock() = port_id;
    Some(port_id)
}

pub fn server_port() -> u32 { *SERVER_PORT.lock() }

// ── Message dispatch ──────────────────────────────────────────────────────────

/// Process one incoming message.
pub fn handle(msg: &Message, reply_port: u32) -> Message {
    match msg.tag {
        PROC_REGISTER    => handle_register(arg(msg,0) as u32, arg(msg,1) as u32,
                                             arg(msg,2) as u32, reply_port),
        PROC_NOTIFY_EXIT => handle_notify_exit(arg(msg,0) as u32, arg(msg,1) as i32),
        PROC_WAIT4       => handle_wait4(arg(msg,0) as u32, reply_port),
        PROC_SETPGID     => handle_setpgid(arg(msg,0) as u32, arg(msg,1) as u32),
        PROC_SETSID      => handle_setsid(arg(msg,0) as u32),
        PROC_GETSID      => handle_getsid(arg(msg,0) as u32),
        PROC_SETCRED     => handle_setcred(arg(msg,0) as u32, arg(msg,1) as u32,
                                            arg(msg,2) as u32),
        _                => err_reply(-38), // ENOSYS
    }
}

fn handle_register(pid: u32, ppid: u32, pgid: u32, reply_port: u32) -> Message {
    let mut tbl = TABLE.lock();
    for slot in tbl.iter_mut() {
        if slot.state == ProcState::Empty {
            *slot = ProcEntry {
                pid, ppid, pgid, sid: pgid,
                uid: 0, gid: 0, euid: 0, egid: 0,
                port: reply_port,
                state: ProcState::Alive,
            };
            return ok_reply();
        }
    }
    err_reply(-12) // ENOMEM
}

fn handle_notify_exit(pid: u32, exit_code: i32) -> Message {
    {
        let mut tbl = TABLE.lock();
        for slot in tbl.iter_mut() {
            if slot.pid == pid && slot.state == ProcState::Alive {
                slot.state = ProcState::Zombie { exit_code };
                break;
            }
        }
    }
    // Wake parked wait4 requests.
    let mut wq = WAIT_QUEUE.lock();
    for slot in wq.iter_mut() {
        if let Some(req) = *slot {
            if req.target_pid == pid || req.target_pid == 0 {
                let mut reply = make_reply(exit_code as i64);
                reply.data[8..16].copy_from_slice(&(pid as u64).to_le_bytes());
                let _ = port::send(req.reply_port, reply);
                *slot = None;
                break;
            }
        }
    }
    ok_reply()
}

fn handle_wait4(target_pid: u32, reply_port: u32) -> Message {
    {
        let tbl = TABLE.lock();
        for slot in tbl.iter() {
            if slot.pid == target_pid || (target_pid == 0 && slot.ppid != 0) {
                if let ProcState::Zombie { exit_code } = slot.state {
                    let mut reply = make_reply(exit_code as i64);
                    reply.data[8..16].copy_from_slice(&(slot.pid as u64).to_le_bytes());
                    return reply;
                }
            }
        }
    }
    // Park the request.
    let req = WaitReq { target_pid, reply_port };
    let mut wq = WAIT_QUEUE.lock();
    for slot in wq.iter_mut() {
        if slot.is_none() {
            *slot = Some(req);
            // Return sentinel: caller blocks on its reply port.
            return make_reply(i64::MIN);
        }
    }
    err_reply(-11) // EAGAIN
}

fn handle_setpgid(pid: u32, pgid: u32) -> Message {
    let mut tbl = TABLE.lock();
    for slot in tbl.iter_mut() {
        if slot.pid == pid { slot.pgid = pgid; return ok_reply(); }
    }
    err_reply(-3)
}

fn handle_setsid(pid: u32) -> Message {
    let mut tbl = TABLE.lock();
    for slot in tbl.iter_mut() {
        if slot.pid == pid {
            slot.sid = pid; slot.pgid = pid;
            return val_reply(pid as u64);
        }
    }
    err_reply(-3)
}

fn handle_getsid(pid: u32) -> Message {
    let tbl = TABLE.lock();
    for slot in tbl.iter() {
        if slot.pid == pid { return val_reply(slot.sid as u64); }
    }
    err_reply(-3)
}

fn handle_setcred(uid: u32, gid: u32, euid: u32) -> Message {
    let pid = sched::current_pid();
    let mut tbl = TABLE.lock();
    for slot in tbl.iter_mut() {
        if slot.pid == pid {
            if slot.euid != 0 && slot.euid != uid { return err_reply(-1); }
            slot.uid = uid; slot.gid = gid; slot.euid = euid;
            return ok_reply();
        }
    }
    err_reply(-3)
}
