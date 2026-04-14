//! Syscall dispatch — the only controlled gate into kernel space.
#![allow(dead_code)] // wired up by arch exception entry stubs (added with user-space)
//!
//! Syscall ABI (register mapping follows Linux on each arch):
//!   AArch64: x8 = number, x0-x5 = args, x0 = return value
//!   x86-64:  rax = number, rdi/rsi/rdx = args, rax = return value
//!
//! Most work is delegated to user-space servers via IPC; the kernel only
//! handles capability/memory/scheduling operations directly.

use ipc::{Message, port};

/// Syscall numbers (ABI-stable once stabilised).
#[repr(usize)]
pub enum Syscall {
    Send     = 0,  // send a message to a port (non-blocking)
    Recv     = 1,  // receive from a port (blocks until message available)
    Call     = 2,  // Send + blocking Recv in one round trip
    MapMem   = 3,
    UnmapMem = 4,
    Yield    = 5,
    Exit     = 6,
}

/// Top-level syscall handler, invoked from the arch-specific trap stub.
///
/// Returns the value to place in the return register (negative = error).
///
/// Exposed as a C symbol so the arch crate can call it without depending on
/// the `kernel` crate directly.
#[no_mangle]
pub extern "C" fn syscall_dispatch(number: usize, a0: usize, a1: usize, a2: usize) -> isize {
    dispatch(number, a0, a1, a2)
}

pub fn dispatch(number: usize, a0: usize, a1: usize, a2: usize) -> isize {
    match number {
        n if n == Syscall::Send    as usize => sys_send(a0, a1, a2),
        n if n == Syscall::Recv    as usize => sys_recv(a0, a1),
        n if n == Syscall::Call    as usize => sys_call(a0, a1, a2),
        n if n == Syscall::Yield   as usize => { sched::yield_now(); 0 }
        n if n == Syscall::Exit    as usize => sched::exit(a0 as i32),
        _ => -1, // ENOSYS
    }
}

/// sys_send(port, msg_ptr, _msg_len) — copy message from caller and enqueue it.
fn sys_send(port_id: usize, msg_ptr: usize, _msg_len: usize) -> isize {
    if msg_ptr == 0 { return -1; }
    // SAFETY: For kernel tasks msg_ptr is a kernel address.
    // For future user tasks: TODO — validate against the process address space.
    let msg = unsafe { core::ptr::read(msg_ptr as *const Message) };
    if port::send(port_id as u32, msg) { 0 } else { -1 }
}

/// sys_recv(port, msg_ptr) — dequeue a message; block if the queue is empty.
fn sys_recv(port_id: usize, msg_ptr: usize) -> isize {
    if msg_ptr == 0 { return -1; }
    loop {
        match port::recv(port_id as u32) {
            Some(msg) => {
                // SAFETY: same caveat as sys_send.
                unsafe { core::ptr::write(msg_ptr as *mut Message, msg); }
                return 0;
            }
            None => sched::block_on(port_id as u32),
        }
    }
}

/// sys_call — atomic send + blocking recv (common IPC pattern).
fn sys_call(port_id: usize, msg_ptr: usize, _msg_len: usize) -> isize {
    let rc = sys_send(port_id, msg_ptr, 0);
    if rc != 0 { return rc; }
    sys_recv(port_id, msg_ptr)
}
