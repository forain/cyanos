//! Syscall dispatch — the only controlled gate into kernel space.
#![allow(dead_code)]
//!
//! Syscall ABI (register mapping follows Linux on each arch):
//!   AArch64: x8 = number, x0-x5 = args, x0 = return value
//!   x86-64:  rax = number, rdi/rsi/rdx = args, rax = return value
//!
//! Most work is delegated to user-space servers via IPC; the kernel only
//! handles capability/memory/scheduling operations directly.

use ipc::{Message, port};
use mm::paging::PageFlags;

/// Upper bound of user-space virtual addresses (canonical hole on 48-bit VA).
const USER_SPACE_END: usize = 0x0000_8000_0000_0000;

/// Validate that `[ptr, ptr+len)` is entirely within user-space.
///
/// Returns true if the range is valid: non-null, no wrap-around, and entirely
/// below the kernel/user split.
fn validate_user_buf(ptr: usize, len: usize) -> bool {
    if ptr == 0 { return false; }
    let end = match ptr.checked_add(len) {
        Some(e) => e,
        None    => return false,  // wrap-around
    };
    end <= USER_SPACE_END
}

/// Validate that `ptr` is in user-space **and** aligned to `align` bytes.
///
/// `align` must be a power of two.  Returns true on success.
/// Used by syscalls that copy typed values (structs, u64) to/from user memory
/// to prevent undefined behaviour from misaligned loads/stores on real hardware.
fn validate_user_ptr_aligned(ptr: usize, size: usize, align: usize) -> bool {
    validate_user_buf(ptr, size) && (ptr & (align - 1)) == 0
}

/// Syscall numbers (ABI-stable once stabilised).
#[repr(usize)]
pub enum Syscall {
    Send         = 0,
    Recv         = 1,
    Call         = 2,
    MapMem       = 3,
    UnmapMem     = 4,
    Yield        = 5,
    Exit         = 6,
    Spawn        = 7,
    ClockGettime = 8,
    Wait         = 9,
}

/// Top-level syscall handler, invoked from the arch-specific trap stub.
///
/// Returns the value to place in the return register (negative = error).
#[no_mangle]
pub extern "C" fn syscall_dispatch(number: usize, a0: usize, a1: usize, a2: usize) -> isize {
    dispatch(number, a0, a1, a2)
}

pub fn dispatch(number: usize, a0: usize, a1: usize, a2: usize) -> isize {
    match number {
        n if n == Syscall::Send    as usize => sys_send(a0, a1, a2),
        n if n == Syscall::Recv    as usize => sys_recv(a0, a1),
        n if n == Syscall::Call    as usize => sys_call(a0, a1, a2),
        n if n == Syscall::MapMem  as usize => sys_map_mem(a0, a1, a2),
        n if n == Syscall::UnmapMem as usize => sys_unmap_mem(a0, a1),
        n if n == Syscall::Yield        as usize => { sched::yield_now(); 0 }
        n if n == Syscall::Exit         as usize => sched::exit(a0 as i32),
        n if n == Syscall::Spawn        as usize => sys_spawn(a0, a1, a2),
        n if n == Syscall::ClockGettime as usize => sys_clock_gettime(a0),
        n if n == Syscall::Wait         as usize => sys_wait(a0, a1),
        _ => -1, // ENOSYS
    }
}

// ── IPC syscalls ──────────────────────────────────────────────────────────────

/// sys_send(port, msg_ptr, _msg_len) — copy message from caller and enqueue it.
fn sys_send(port_id: usize, msg_ptr: usize, _msg_len: usize) -> isize {
    // Message must be naturally aligned (8-byte) so the read is defined.
    if !validate_user_ptr_aligned(msg_ptr, core::mem::size_of::<Message>(), 8) { return -14; }
    let msg = unsafe { core::ptr::read(msg_ptr as *const Message) };
    match port::send(port_id as u32, msg) {
        Ok(())                          =>  0,
        Err(port::SendError::QueueFull) => -11, // EAGAIN — queue full, caller should retry
        Err(port::SendError::PortNotFound) => -9, // EBADF — invalid port
    }
}

/// sys_recv(port, msg_ptr) — dequeue a message; block if the queue is empty.
///
/// Returns:
///   -13 (EACCES) — the calling task does not own the port
///   -9  (EBADF)  — port was closed while the task was blocked (woken by
///                  `release_by_owner` → `sched::unblock_port`)
fn sys_recv(port_id: usize, msg_ptr: usize) -> isize {
    // Message must be naturally aligned (8-byte) so the write is defined.
    if !validate_user_ptr_aligned(msg_ptr, core::mem::size_of::<Message>(), 8) { return -14; }
    let caller = sched::current_pid();
    if !port::is_owner(port_id as u32, caller) { return -13; }  // EACCES
    loop {
        match port::recv_as(port_id as u32, caller) {
            Some(msg) => {
                unsafe { core::ptr::write(msg_ptr as *mut Message, msg); }
                return 0;
            }
            None => {
                // Check whether the port still exists before blocking.
                // It may have been closed by release_by_owner between the
                // ownership check above and this point.
                if !port::is_owner(port_id as u32, caller) {
                    return -9; // EBADF — port was closed
                }
                sched::block_on(port_id as u32);
                // After being woken (either by a send or by release_by_owner),
                // re-check port existence before looping back to recv_as.
                if !port::is_owner(port_id as u32, caller) {
                    return -9; // EBADF — port closed while we were blocked
                }
            }
        }
    }
}

/// sys_call — send to `port_id`, then block on the caller's own reply port.
///
/// The reply port is lazily allocated on the first call and cached in the
/// `Task::reply_port` field.  The port ID is stamped into `msg.reply_port`
/// before the message is forwarded, so the server can send its response back
/// to the correct endpoint via `sys_send(msg.reply_port, reply_msg)`.
///
/// Unlike the old implementation, the caller waits on a port it **owns**
/// rather than on the server's port, fixing the EACCES ownership error.
fn sys_call(port_id: usize, msg_ptr: usize, _msg_len: usize) -> isize {
    if !validate_user_ptr_aligned(msg_ptr, core::mem::size_of::<Message>(), 8) { return -14; }

    // Lazily allocate the caller's reply port.
    let reply_port = {
        let rp = sched::current_reply_port();
        if rp != u32::MAX {
            rp
        } else {
            let caller = sched::current_pid();
            match port::create(caller) {
                Some(p) => { sched::set_current_reply_port(p); p }
                None    => return -12, // ENOMEM — port table full
            }
        }
    };

    // Read the message, stamp our reply port, and forward it to the server.
    let mut msg = unsafe { core::ptr::read(msg_ptr as *const Message) };
    msg.reply_port = reply_port;
    match port::send(port_id as u32, msg) {
        Ok(())                              => {}
        Err(port::SendError::QueueFull)     => return -11, // EAGAIN
        Err(port::SendError::PortNotFound)  => return -9,  // EBADF
    }

    // Block on our own reply port (which we own) until the server responds.
    sys_recv(reply_port as usize, msg_ptr)
}

// ── Memory syscalls ───────────────────────────────────────────────────────────

/// Maximum bytes a single sys_map_mem call may request.
/// Prevents a user task from exhausting the buddy allocator in one call.
const MAP_MAX_BYTES: usize = 256 * 1024 * 1024; // 256 MiB

/// sys_map_mem(virt, size, flags) — map `size` bytes at `virt` in the calling
/// task's address space.
///
/// `flags` bits match `mm::paging::PageFlags`:
///   bit 0 = PRESENT, bit 1 = WRITABLE, bit 2 = USER, bit 3 = EXECUTE
///
/// Returns 0 on success, negative errno on failure.
fn sys_map_mem(virt: usize, size: usize, flags_bits: usize) -> isize {
    if virt == 0 || size == 0 { return -22; } // EINVAL
    if size > MAP_MAX_BYTES    { return -22; } // EINVAL — cap per-call allocation

    // The target range must be entirely in user space.
    let end = match virt.checked_add(size) {
        Some(e) => e,
        None    => return -22,
    };
    if end > USER_SPACE_END { return -22; }

    let flags = PageFlags::from_bits_truncate(flags_bits as u64)
        | PageFlags::PRESENT
        | PageFlags::USER;

    // W^X: reject mappings that are both writable and executable.
    if flags.contains(PageFlags::WRITABLE) && flags.contains(PageFlags::EXECUTE) {
        return -22; // EINVAL
    }

    let ok = sched::with_current_address_space(|as_| as_.map(virt, size, flags));
    match ok {
        Some(true)  =>  0,
        Some(false) => -12, // ENOMEM
        None        => -1,  // no address space (kernel task)
    }
}

/// sys_unmap_mem(virt, size) — unmap and free the pages at `virt`.
fn sys_unmap_mem(virt: usize, size: usize) -> isize {
    if virt == 0 || size == 0 { return -22; } // EINVAL
    if virt >= USER_SPACE_END  { return -22; }

    sched::with_current_address_space(|as_| as_.unmap(virt, size));
    0
}

// ── Task management syscalls ──────────────────────────────────────────────────

/// sys_spawn(entry_va, stack_va, priority) — spawn a user-mode task.
///
/// `entry_va`  — virtual address of the task entry point (must be in user space)
/// `stack_va`  — virtual address of the top of the user stack
/// `priority`  — signed 8-bit scheduling priority, passed as a `usize`
///               (cast to `i8`; callers typically pass 0 for normal priority)
///
/// Returns the new task's PID (positive), or a negative errno on failure:
///   -22 (EINVAL)  — entry_va or stack_va is outside user space
///   -12 (ENOMEM)  — run queue full or OOM
fn sys_spawn(entry_va: usize, stack_va: usize, priority_raw: usize) -> isize {
    // Reject entries that point into the kernel half of the address space.
    if entry_va == 0 || entry_va >= USER_SPACE_END { return -22; }
    if stack_va  >= USER_SPACE_END                 { return -22; }

    let priority = priority_raw as i8;
    match sched::spawn_user(entry_va, stack_va, priority) {
        Some(pid) => pid as isize,
        None      => -12, // ENOMEM
    }
}

/// sys_wait(pid, status_ptr) — block until `pid` exits; write its exit code.
///
/// Blocks until the target task becomes a Zombie, writes its `i32` exit code
/// to `status_ptr` (user-space aligned pointer), reaps the task, and returns 0.
///
/// Returns:
///   -3  (ESRCH)   — `pid` does not exist
///   -14 (EFAULT)  — `status_ptr` is null, misaligned, or out of range
fn sys_wait(pid_raw: usize, status_ptr: usize) -> isize {
    // Validate before blocking — catches bad pointers before we yield.
    if !validate_user_ptr_aligned(status_ptr, core::mem::size_of::<i32>(), 4) { return -14; }

    match sched::wait_pid(pid_raw as u32) {
        Some(code) => {
            unsafe { core::ptr::write(status_ptr as *mut i32, code); }
            0
        }
        None => -3, // ESRCH — pid not found
    }
}

/// sys_clock_gettime(dest_ptr) — write monotonic tick counter to user memory.
///
/// Writes the current 64-bit tick counter (`sched::ticks()`) as a little-endian
/// `u64` at the user-space address `dest_ptr`.
///
/// Returns 0 on success, or:
///   -14 (EFAULT) — `dest_ptr` is null, wraps, or outside user space
fn sys_clock_gettime(dest_ptr: usize) -> isize {
    // u64 requires 8-byte alignment; misaligned write is UB on real hardware.
    if !validate_user_ptr_aligned(dest_ptr, core::mem::size_of::<u64>(), 8) { return -14; }
    let ticks = sched::ticks();
    unsafe { core::ptr::write(dest_ptr as *mut u64, ticks); }
    0
}
