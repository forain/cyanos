//! TTY server — POSIX terminal (termios) line discipline and PTY pairs.
//!
//! # Architecture
//!
//! In-kernel library called directly from syscall.rs for ioctl(TCGETS/TCSETS),
//! isatty(), and read/write on TTY fds.  /dev/tty and /dev/ttyS0 are VFS devfs
//! vnodes whose read/write handlers delegate here.
//!
//! TTY FDs use the same SOCK_FD_BASE offset scheme as the net server but with
//! a distinct range: TTY_FD_BASE = 0x200.
//!
//! # POSIX timers
//!
//! `timer_create`/`timer_settime`/`timer_gettime`/`timer_delete` are
//! implemented here as a small per-process timer table.  Each timer is a
//! deadline (in scheduler ticks) checked on every syscall return and yielded
//! tick.  Expiry fires by calling `sched::deliver_signal`.
//!
//! # Message encoding
//!
//! Arguments packed as little-endian u64 words in Message.data[], as in VFS.

#![no_std]

use ipc::Message;
use spin::Mutex;

// ── Protocol tag constants ────────────────────────────────────────────────────

pub const TTY_OPEN:       u64 = 0x40;
pub const TTY_READ:       u64 = 0x41;
pub const TTY_WRITE:      u64 = 0x42;
pub const TTY_IOCTL:      u64 = 0x43;
pub const TTY_CLOSE:      u64 = 0x44;
pub const TTY_ISATTY:     u64 = 0x45;

// POSIX timer protocol
pub const TIMER_CREATE:   u64 = 0x50;
pub const TIMER_SETTIME:  u64 = 0x51;
pub const TIMER_GETTIME:  u64 = 0x52;
pub const TIMER_DELETE:   u64 = 0x53;

// ── Constants ─────────────────────────────────────────────────────────────────

pub const TTY_FD_BASE: usize = 0x200;

const MAX_PROCS:  usize = 64;
const MAX_TTYS:   usize = 4;   // per process (stdin/stdout/stderr + one pty)
const MAX_TIMERS: usize = 8;   // per process POSIX timers
const TTY_BUF:    usize = 4096;

// ── termios structure (matches Linux struct termios) ──────────────────────────
// 36 bytes: c_iflag(4)+c_oflag(4)+c_cflag(4)+c_lflag(4)+c_line(1)+[3 pad]+c_cc[19]+[1 pad]
// For simplicity we store as 60 bytes (termios2 / larger buffer).

#[derive(Clone, Copy)]
struct Termios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line:  u8,
    c_cc:    [u8; 19],
}

impl Termios {
    /// Return a sensible default for a serial console.
    const fn default_console() -> Self {
        let mut cc = [0u8; 19];
        cc[2]  = 0x7F; // VERASE = DEL
        cc[3]  = 0x15; // VKILL  = Ctrl-U
        cc[4]  = 4;    // VEOF   = Ctrl-D (min bytes for raw read)
        cc[7]  = 0;    // VSTART
        cc[8]  = 0;    // VSTOP
        cc[9]  = 0x1A; // VSUSP  = Ctrl-Z
        cc[11] = 0x11; // VREPRINT = Ctrl-Q
        cc[12] = 0x12; // VDISCARD = Ctrl-R
        cc[13] = 0x17; // VWERASE  = Ctrl-W
        cc[14] = 0x16; // VLNEXT   = Ctrl-V
        Self {
            c_iflag: 0x0500, // ICRNL | IXON
            c_oflag: 0x0005, // OPOST | ONLCR
            c_cflag: 0x04BF, // CS8 | CREAD | HUPCL | CLOCAL
            c_lflag: 0x8A3B, // ISIG|ICANON|ECHO|ECHOE|ECHOK|IEXTEN|ECHOCTL|ECHOKE
            c_line:  0,
            c_cc:    cc,
        }
    }
}

// ── TTY ring buffer ───────────────────────────────────────────────────────────

struct TtyBuf {
    data:  [u8; TTY_BUF],
    rpos:  usize,
    wpos:  usize,
    count: usize,
}

impl TtyBuf {
    const fn new() -> Self {
        Self { data: [0u8; TTY_BUF], rpos: 0, wpos: 0, count: 0 }
    }

    fn write(&mut self, b: u8) -> bool {
        if self.count >= TTY_BUF { return false; }
        self.data[self.wpos] = b;
        self.wpos = (self.wpos + 1) % TTY_BUF;
        self.count += 1;
        true
    }

    fn read(&mut self) -> Option<u8> {
        if self.count == 0 { return None; }
        let b = self.data[self.rpos];
        self.rpos = (self.rpos + 1) % TTY_BUF;
        self.count -= 1;
        Some(b)
    }
}

// ── TTY entry ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum TtyKind {
    None,
    Console,  // /dev/ttyS0 — backed by kernel serial I/O
    PtsMaster { pair: usize },
    PtsSlave  { pair: usize },
}

#[derive(Clone, Copy)]
struct TtyEntry {
    kind:   TtyKind,
    in_use: bool,
    termios: Termios,
}

impl TtyEntry {
    const fn empty() -> Self {
        Self { kind: TtyKind::None, in_use: false, termios: Termios::default_console() }
    }
}

#[derive(Clone, Copy)]
struct ProcTtyTable {
    pid:    u32,
    ttys:   [TtyEntry; MAX_TTYS],
    in_use: bool,
}

impl ProcTtyTable {
    const fn empty() -> Self {
        Self { pid: 0, ttys: [const { TtyEntry::empty() }; MAX_TTYS], in_use: false }
    }

    fn alloc(&mut self) -> Option<usize> {
        self.ttys.iter().position(|t| !t.in_use)
    }
}

static TTY_TABLES: Mutex<[ProcTtyTable; MAX_PROCS]> =
    Mutex::new([const { ProcTtyTable::empty() }; MAX_PROCS]);

// ── PTY pairs ────────────────────────────────────────────────────────────────

const MAX_PTS: usize = 8;

struct PtsPair {
    in_use:   bool,
    m_to_s:   TtyBuf,  // master→slave (slave reads this)
    s_to_m:   TtyBuf,  // slave→master (master reads this)
}

impl PtsPair {
    const fn new() -> Self {
        Self { in_use: false, m_to_s: TtyBuf::new(), s_to_m: TtyBuf::new() }
    }
}

static PTS_PAIRS: Mutex<[PtsPair; MAX_PTS]> =
    Mutex::new([const { PtsPair::new() }; MAX_PTS]);

// ── POSIX timer table ─────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct PosixTimer {
    in_use:    bool,
    signo:     u32,
    interval:  u64, // repeat interval in ticks (0 = one-shot)
    deadline:  u64, // absolute tick deadline (0 = disarmed)
    owner_pid: u32,
}

impl PosixTimer {
    const fn new() -> Self {
        Self { in_use: false, signo: 0, interval: 0, deadline: 0, owner_pid: 0 }
    }
}

#[derive(Clone, Copy)]
struct ProcTimerTable {
    pid:    u32,
    timers: [PosixTimer; MAX_TIMERS],
    in_use: bool,
}

impl ProcTimerTable {
    const fn empty() -> Self {
        Self { pid: 0, timers: [const { PosixTimer::new() }; MAX_TIMERS], in_use: false }
    }

    fn alloc(&mut self) -> Option<usize> {
        self.timers.iter().position(|t| !t.in_use)
    }
}

static TIMER_TABLES: Mutex<[ProcTimerTable; MAX_PROCS]> =
    Mutex::new([const { ProcTimerTable::empty() }; MAX_PROCS]);

// ── Helpers ───────────────────────────────────────────────────────────────────

fn arg(msg: &Message, n: usize) -> u64 {
    let off = n * 8;
    u64::from_le_bytes(msg.data[off..off + 8].try_into().unwrap_or([0u8; 8]))
}

fn make_reply(v: i64) -> Message {
    let mut m = Message::empty();
    m.data[0..8].copy_from_slice(&(v as u64).to_le_bytes());
    m
}

fn ok_reply()        -> Message { make_reply(0) }
fn err_reply(e: i32) -> Message { make_reply(e as i64) }
fn val_reply(v: u64) -> Message { make_reply(v as i64) }

fn find_tty<'a>(pid: u32, tbls: &'a mut [ProcTtyTable]) -> Option<&'a mut ProcTtyTable> {
    tbls.iter_mut().find(|t| t.in_use && t.pid == pid)
}

fn get_or_create_tty<'a>(pid: u32, tbls: &'a mut [ProcTtyTable]) -> Option<&'a mut ProcTtyTable> {
    if let Some(pos) = tbls.iter().position(|t| t.in_use && t.pid == pid) {
        return Some(&mut tbls[pos]);
    }
    if let Some(pos) = tbls.iter().position(|t| !t.in_use) {
        tbls[pos] = ProcTtyTable::empty();
        tbls[pos].in_use = true;
        tbls[pos].pid    = pid;
        return Some(&mut tbls[pos]);
    }
    None
}

fn fd_to_slot(fd: usize) -> Option<usize> {
    if fd >= TTY_FD_BASE && fd < TTY_FD_BASE + MAX_TTYS { Some(fd - TTY_FD_BASE) } else { None }
}

// ── Public dispatch ───────────────────────────────────────────────────────────

pub fn handle(msg: &Message, caller_pid: u32) -> Message {
    match msg.tag {
        TTY_OPEN    => handle_open(caller_pid, arg(msg,0) as usize),
        TTY_READ    => handle_read(caller_pid, arg(msg,0) as usize,
                                   arg(msg,1) as usize, arg(msg,2) as usize),
        TTY_WRITE   => handle_write(caller_pid, arg(msg,0) as usize,
                                    arg(msg,1) as usize, arg(msg,2) as usize),
        TTY_IOCTL   => handle_ioctl(caller_pid, arg(msg,0) as usize,
                                    arg(msg,1) as usize, arg(msg,2) as usize),
        TTY_CLOSE   => handle_close(caller_pid, arg(msg,0) as usize),
        TTY_ISATTY  => handle_isatty(caller_pid, arg(msg,0) as usize),
        TIMER_CREATE  => handle_timer_create(caller_pid, arg(msg,0) as u32,
                                             arg(msg,1) as usize),
        TIMER_SETTIME => handle_timer_settime(caller_pid, arg(msg,0) as usize,
                                              arg(msg,1) as usize, arg(msg,2) as usize),
        TIMER_GETTIME => handle_timer_gettime(caller_pid, arg(msg,0) as usize,
                                              arg(msg,1) as usize),
        TIMER_DELETE  => handle_timer_delete(caller_pid, arg(msg,0) as usize),
        _ => err_reply(-38),
    }
}

/// Check and fire expired POSIX timers for `pid`.  Called on syscall return.
pub fn check_timers(pid: u32) {
    let now = sched::ticks();
    let mut tbls = TIMER_TABLES.lock();
    let tbl = match tbls.iter_mut().find(|t| t.in_use && t.pid == pid) {
        Some(t) => t, None => return,
    };
    for timer in tbl.timers.iter_mut() {
        if !timer.in_use || timer.deadline == 0 { continue; }
        if now >= timer.deadline {
            sched::deliver_signal(timer.owner_pid, timer.signo);
            if timer.interval > 0 {
                timer.deadline += timer.interval;
            } else {
                timer.deadline = 0; // disarm one-shot
            }
        }
    }
}

/// Drop all TTY and timer state for a process on exit.
pub fn close_all(pid: u32) {
    let mut ttys = TTY_TABLES.lock();
    if let Some(t) = ttys.iter_mut().find(|t| t.in_use && t.pid == pid) {
        *t = ProcTtyTable::empty();
    }
    let mut timers = TIMER_TABLES.lock();
    if let Some(t) = timers.iter_mut().find(|t| t.in_use && t.pid == pid) {
        *t = ProcTimerTable::empty();
    }
}

// ── TTY handlers ──────────────────────────────────────────────────────────────

fn handle_open(pid: u32, kind_raw: usize) -> Message {
    // kind_raw: 0=console(/dev/ttyS0), 1=pts master, 2=pts slave
    let kind = match kind_raw {
        0 => TtyKind::Console,
        1 => {
            let pair = {
                let mut pairs = PTS_PAIRS.lock();
                match pairs.iter().position(|p| !p.in_use) {
                    Some(i) => { pairs[i].in_use = true; i }
                    None    => return err_reply(-24),
                }
            };
            TtyKind::PtsMaster { pair }
        }
        2 => {
            // Slave must be opened after master; pair index carried in kind_raw>>8.
            let pair = kind_raw >> 8;
            TtyKind::PtsSlave { pair }
        }
        _ => return err_reply(-22),
    };
    let mut tbls = TTY_TABLES.lock();
    let tbl = match get_or_create_tty(pid, &mut *tbls) {
        Some(t) => t, None => return err_reply(-12),
    };
    let slot = match tbl.alloc() { Some(s) => s, None => return err_reply(-24) };
    tbl.ttys[slot] = TtyEntry { kind, in_use: true, termios: Termios::default_console() };
    val_reply((slot + TTY_FD_BASE) as u64)
}

fn handle_read(pid: u32, fd: usize, buf_ptr: usize, count: usize) -> Message {
    let slot = match fd_to_slot(fd) { Some(s) => s, None => return err_reply(-9) };
    let tbls = TTY_TABLES.lock();
    let tbl = match tbls.iter().find(|t| t.in_use && t.pid == pid) {
        Some(t) => t, None => return err_reply(-9),
    };
    if slot >= MAX_TTYS || !tbl.ttys[slot].in_use { return err_reply(-9); }
    match tbl.ttys[slot].kind {
        TtyKind::Console => {
            // No input buffer yet — return EOF (0).
            val_reply(0)
        }
        TtyKind::PtsSlave { pair } => {
            drop(tbls);
            let mut pairs = PTS_PAIRS.lock();
            let mut n = 0usize;
            let ring = &mut pairs[pair].m_to_s;
            while n < count.min(TTY_BUF) {
                match ring.read() {
                    Some(b) => { unsafe { *(buf_ptr as *mut u8).add(n) = b; } n += 1; }
                    None    => break,
                }
            }
            val_reply(n as u64)
        }
        TtyKind::PtsMaster { pair } => {
            drop(tbls);
            let mut pairs = PTS_PAIRS.lock();
            let mut n = 0usize;
            let ring = &mut pairs[pair].s_to_m;
            while n < count.min(TTY_BUF) {
                match ring.read() {
                    Some(b) => { unsafe { *(buf_ptr as *mut u8).add(n) = b; } n += 1; }
                    None    => break,
                }
            }
            val_reply(n as u64)
        }
        TtyKind::None => err_reply(-9),
    }
}

fn handle_write(pid: u32, fd: usize, buf_ptr: usize, count: usize) -> Message {
    let slot = match fd_to_slot(fd) { Some(s) => s, None => return err_reply(-9) };
    let tbls = TTY_TABLES.lock();
    let tbl = match tbls.iter().find(|t| t.in_use && t.pid == pid) {
        Some(t) => t, None => return err_reply(-9),
    };
    if slot >= MAX_TTYS || !tbl.ttys[slot].in_use { return err_reply(-9); }
    match tbl.ttys[slot].kind {
        TtyKind::Console => {
            // Write to serial output.
            let buf = buf_ptr as *const u8;
            for i in 0..count.min(4096) {
                let b = unsafe { *buf.add(i) };
                // Route through kernel serial — use the extern fn via a raw
                // byte cast to avoid dependency on kernel crate from here.
                // The caller (syscall.rs) handles serial; for now just count.
                let _ = b;
            }
            val_reply(count as u64)
        }
        TtyKind::PtsMaster { pair } => {
            drop(tbls);
            let mut pairs = PTS_PAIRS.lock();
            let mut n = 0usize;
            let ring = &mut pairs[pair].m_to_s;
            let buf = buf_ptr as *const u8;
            while n < count { if !ring.write(unsafe { *buf.add(n) }) { break; } n += 1; }
            val_reply(n as u64)
        }
        TtyKind::PtsSlave { pair } => {
            drop(tbls);
            let mut pairs = PTS_PAIRS.lock();
            let mut n = 0usize;
            let ring = &mut pairs[pair].s_to_m;
            let buf = buf_ptr as *const u8;
            while n < count { if !ring.write(unsafe { *buf.add(n) }) { break; } n += 1; }
            val_reply(n as u64)
        }
        TtyKind::None => err_reply(-9),
    }
}

fn handle_ioctl(pid: u32, fd: usize, cmd: usize, arg_ptr: usize) -> Message {
    // Linux ioctl commands for termios:
    // TCGETS  = 0x5401
    // TCSETS  = 0x5402
    // TCSETSW = 0x5403
    // TCSETSF = 0x5404
    // TIOCGWINSZ = 0x5413
    const TCGETS:     usize = 0x5401;
    const TCSETS:     usize = 0x5402;
    const TCSETSW:    usize = 0x5403;
    const TCSETSF:    usize = 0x5404;
    const TIOCGWINSZ: usize = 0x5413;

    let slot = match fd_to_slot(fd) { Some(s) => s, None => return err_reply(-25) }; // ENOTTY
    let mut tbls = TTY_TABLES.lock();
    let tbl = match find_tty(pid, &mut *tbls) { Some(t) => t, None => return err_reply(-25) };
    if slot >= MAX_TTYS || !tbl.ttys[slot].in_use { return err_reply(-25); }

    match cmd {
        TCGETS => {
            // struct termios is 36 bytes on Linux.  We write our fields.
            if arg_ptr == 0 { return err_reply(-14); }
            let t = &tbl.ttys[slot].termios;
            unsafe {
                core::ptr::write(arg_ptr           as *mut u32, t.c_iflag);
                core::ptr::write((arg_ptr + 4)     as *mut u32, t.c_oflag);
                core::ptr::write((arg_ptr + 8)     as *mut u32, t.c_cflag);
                core::ptr::write((arg_ptr + 12)    as *mut u32, t.c_lflag);
                core::ptr::write((arg_ptr + 16)    as *mut u8,  t.c_line);
                core::ptr::copy_nonoverlapping(t.c_cc.as_ptr(), (arg_ptr + 17) as *mut u8, 19);
            }
            ok_reply()
        }
        TCSETS | TCSETSW | TCSETSF => {
            if arg_ptr == 0 { return err_reply(-14); }
            let t = &mut tbl.ttys[slot].termios;
            unsafe {
                t.c_iflag = core::ptr::read(arg_ptr         as *const u32);
                t.c_oflag = core::ptr::read((arg_ptr + 4)   as *const u32);
                t.c_cflag = core::ptr::read((arg_ptr + 8)   as *const u32);
                t.c_lflag = core::ptr::read((arg_ptr + 12)  as *const u32);
                t.c_line  = core::ptr::read((arg_ptr + 16)  as *const u8);
                core::ptr::copy_nonoverlapping((arg_ptr + 17) as *const u8,
                                               t.c_cc.as_mut_ptr(), 19);
            }
            ok_reply()
        }
        TIOCGWINSZ => {
            // struct winsize { ws_row, ws_col, ws_xpixel, ws_ypixel } — 4×u16 = 8 bytes
            if arg_ptr == 0 { return err_reply(-14); }
            unsafe {
                core::ptr::write(arg_ptr       as *mut u16, 24);  // rows
                core::ptr::write((arg_ptr + 2) as *mut u16, 80);  // cols
                core::ptr::write((arg_ptr + 4) as *mut u16, 0);
                core::ptr::write((arg_ptr + 6) as *mut u16, 0);
            }
            ok_reply()
        }
        _ => err_reply(-25), // ENOTTY — unknown ioctl
    }
}

fn handle_close(pid: u32, fd: usize) -> Message {
    let slot = match fd_to_slot(fd) { Some(s) => s, None => return err_reply(-9) };
    let mut tbls = TTY_TABLES.lock();
    if let Some(tbl) = find_tty(pid, &mut *tbls) {
        if slot < MAX_TTYS && tbl.ttys[slot].in_use {
            // Close PTY master → free the pair.
            if let TtyKind::PtsMaster { pair } = tbl.ttys[slot].kind {
                drop(tbls);
                PTS_PAIRS.lock()[pair].in_use = false;
                return ok_reply();
            }
            tbl.ttys[slot] = TtyEntry::empty();
        }
    }
    ok_reply()
}

fn handle_isatty(pid: u32, fd: usize) -> Message {
    let slot = match fd_to_slot(fd) { Some(s) => s, None => return val_reply(0) };
    let tbls = TTY_TABLES.lock();
    let tbl = match tbls.iter().find(|t| t.in_use && t.pid == pid) {
        Some(t) => t, None => return val_reply(0),
    };
    if slot < MAX_TTYS && tbl.ttys[slot].in_use && tbl.ttys[slot].kind != TtyKind::None {
        val_reply(1)
    } else {
        val_reply(0)
    }
}

// ── POSIX timer handlers ──────────────────────────────────────────────────────

fn handle_timer_create(pid: u32, signo: u32, timerid_ptr: usize) -> Message {
    let mut tbls = TIMER_TABLES.lock();
    let tbl = {
        let pos = if let Some(p) = tbls.iter().position(|t| t.in_use && t.pid == pid) { p }
                  else if let Some(p) = tbls.iter().position(|t| !t.in_use) { p }
                  else { return err_reply(-12); };
        tbls[pos].in_use = true;
        tbls[pos].pid    = pid;
        &mut tbls[pos]
    };
    let slot = match tbl.alloc() { Some(s) => s, None => return err_reply(-11) };
    tbl.timers[slot] = PosixTimer { in_use: true, signo, interval: 0, deadline: 0,
                                    owner_pid: pid };
    if timerid_ptr != 0 {
        unsafe { core::ptr::write(timerid_ptr as *mut u32, slot as u32); }
    }
    ok_reply()
}

fn handle_timer_settime(pid: u32, timerid: usize, ispec_ptr: usize, _ospec_ptr: usize)
    -> Message
{
    // struct itimerspec: { it_interval: timespec, it_value: timespec }
    // struct timespec:   { tv_sec: i64, tv_nsec: i64 } (16 bytes each)
    // Total: 32 bytes.  We convert to scheduler ticks (100 Hz assumed).
    const TICK_HZ: u64 = 100;

    if ispec_ptr == 0 { return err_reply(-14); }
    let interval_sec  = unsafe { core::ptr::read(ispec_ptr          as *const i64) };
    let interval_nsec = unsafe { core::ptr::read((ispec_ptr + 8)    as *const i64) };
    let value_sec     = unsafe { core::ptr::read((ispec_ptr + 16)   as *const i64) };
    let value_nsec    = unsafe { core::ptr::read((ispec_ptr + 24)   as *const i64) };

    let interval_ticks = (interval_sec as u64 * TICK_HZ)
                       + (interval_nsec as u64 / (1_000_000_000 / TICK_HZ));
    let value_ticks    = (value_sec as u64 * TICK_HZ)
                       + (value_nsec as u64 / (1_000_000_000 / TICK_HZ));

    let mut tbls = TIMER_TABLES.lock();
    if let Some(tbl) = tbls.iter_mut().find(|t| t.in_use && t.pid == pid) {
        if timerid < MAX_TIMERS && tbl.timers[timerid].in_use {
            tbl.timers[timerid].interval = interval_ticks;
            tbl.timers[timerid].deadline = if value_ticks > 0 {
                sched::ticks() + value_ticks
            } else {
                0 // disarm
            };
        }
    }
    ok_reply()
}

fn handle_timer_gettime(pid: u32, timerid: usize, ospec_ptr: usize) -> Message {
    const TICK_HZ: u64 = 100;
    const NSEC_PER_TICK: u64 = 1_000_000_000 / TICK_HZ;

    if ospec_ptr == 0 { return err_reply(-14); }
    let tbls = TIMER_TABLES.lock();
    if let Some(tbl) = tbls.iter().find(|t| t.in_use && t.pid == pid) {
        if timerid < MAX_TIMERS && tbl.timers[timerid].in_use {
            let interval_ticks = tbl.timers[timerid].interval;
            let remaining = {
                let dl = tbl.timers[timerid].deadline;
                let now = sched::ticks();
                if dl > now { dl - now } else { 0 }
            };
            unsafe {
                // it_interval
                core::ptr::write(ospec_ptr          as *mut i64,
                                 (interval_ticks / TICK_HZ) as i64);
                core::ptr::write((ospec_ptr + 8)    as *mut i64,
                                 ((interval_ticks % TICK_HZ) * NSEC_PER_TICK) as i64);
                // it_value (remaining)
                core::ptr::write((ospec_ptr + 16)   as *mut i64,
                                 (remaining / TICK_HZ) as i64);
                core::ptr::write((ospec_ptr + 24)   as *mut i64,
                                 ((remaining % TICK_HZ) * NSEC_PER_TICK) as i64);
            }
            return ok_reply();
        }
    }
    err_reply(-22)
}

fn handle_timer_delete(pid: u32, timerid: usize) -> Message {
    let mut tbls = TIMER_TABLES.lock();
    if let Some(tbl) = tbls.iter_mut().find(|t| t.in_use && t.pid == pid) {
        if timerid < MAX_TIMERS { tbl.timers[timerid] = PosixTimer::new(); }
    }
    ok_reply()
}
