//! Syscall dispatch — the only controlled gate into kernel space.
#![allow(dead_code, unused_imports)]
//!
//! Like Linux, syscalls are numbered. Unlike a monolithic kernel, most
//! work is delegated to user-space servers via IPC.

use ipc::Message;

/// Syscall numbers (ABI-stable once stabilised).
#[repr(usize)]
pub enum Syscall {
    Send    = 0,
    Recv    = 1,
    Call    = 2,  // Send + blocking Recv in one operation.
    MapMem  = 3,
    UnmapMem = 4,
    Yield   = 5,
    Exit    = 6,
}

/// Top-level syscall handler, invoked from the arch-specific interrupt stub.
pub fn dispatch(number: usize, a0: usize, a1: usize, a2: usize) -> isize {
    match number {
        n if n == Syscall::Send as usize   => sys_send(a0, a1, a2),
        n if n == Syscall::Recv as usize   => sys_recv(a0, a1),
        n if n == Syscall::Yield as usize  => { sched::r#yield(); 0 }
        n if n == Syscall::Exit as usize   => sched::exit(a0 as i32),
        _ => -1, // ENOSYS
    }
}

fn sys_send(port: usize, msg_ptr: usize, msg_len: usize) -> isize {
    // Safety: caller must ensure ptr/len are valid userspace mappings.
    let _ = (port, msg_ptr, msg_len);
    0
}

fn sys_recv(port: usize, msg_ptr: usize) -> isize {
    let _ = (port, msg_ptr);
    0
}
