//! Raw Linux-ABI syscall wrappers for AArch64.
//!
//! Calling convention: x8 = syscall number, x0–x5 = arguments, svc #0,
//! return value in x0 (negative errno on error).

#[cfg(target_arch = "aarch64")]
use core::arch::asm;

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub unsafe fn syscall0(nr: usize) -> isize {
    let ret: isize;
    asm!("svc #0", in("x8") nr, lateout("x0") ret, options(nostack));
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub unsafe fn syscall1(nr: usize, a0: usize) -> isize {
    let ret: isize;
    asm!("svc #0", in("x8") nr, inlateout("x0") a0 => ret, options(nostack));
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub unsafe fn syscall2(nr: usize, a0: usize, a1: usize) -> isize {
    let ret: isize;
    asm!("svc #0", in("x8") nr, inlateout("x0") a0 => ret,
         in("x1") a1, options(nostack));
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub unsafe fn syscall3(nr: usize, a0: usize, a1: usize, a2: usize) -> isize {
    let ret: isize;
    asm!("svc #0", in("x8") nr, inlateout("x0") a0 => ret,
         in("x1") a1, in("x2") a2, options(nostack));
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub unsafe fn syscall4(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    asm!("svc #0", in("x8") nr, inlateout("x0") a0 => ret,
         in("x1") a1, in("x2") a2, in("x3") a3, options(nostack));
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub unsafe fn syscall6(nr: usize, a0: usize, a1: usize, a2: usize,
                       a3: usize, a4: usize, a5: usize) -> isize {
    let ret: isize;
    asm!("svc #0", in("x8") nr, inlateout("x0") a0 => ret,
         in("x1") a1, in("x2") a2, in("x3") a3,
         in("x4") a4, in("x5") a5, options(nostack));
    ret
}

// Stubs for non-AArch64 hosts (lets `cargo check` pass on the Android host).
#[cfg(not(target_arch = "aarch64"))]
pub unsafe fn syscall0(_nr: usize) -> isize { 0 }
#[cfg(not(target_arch = "aarch64"))]
pub unsafe fn syscall1(_nr: usize, _a0: usize) -> isize { 0 }
#[cfg(not(target_arch = "aarch64"))]
pub unsafe fn syscall2(_nr: usize, _a0: usize, _a1: usize) -> isize { 0 }
#[cfg(not(target_arch = "aarch64"))]
pub unsafe fn syscall3(_nr: usize, _a0: usize, _a1: usize, _a2: usize) -> isize { 0 }
#[cfg(not(target_arch = "aarch64"))]
pub unsafe fn syscall4(_nr: usize, _a0: usize, _a1: usize, _a2: usize, _a3: usize) -> isize { 0 }
#[cfg(not(target_arch = "aarch64"))]
pub unsafe fn syscall6(_nr: usize, _a0: usize, _a1: usize, _a2: usize,
                       _a3: usize, _a4: usize, _a5: usize) -> isize { 0 }

/// Linux AArch64 syscall numbers (identical to those Cyanos uses).
pub mod nr {
    pub const READ:          usize = 63;
    pub const WRITE:         usize = 64;
    pub const OPENAT:        usize = 56;
    pub const CLOSE:         usize = 57;
    pub const LSEEK:         usize = 62;
    pub const MMAP:          usize = 222;
    pub const MUNMAP:        usize = 215;
    pub const BRK:           usize = 214;
    pub const CLONE:         usize = 220;
    pub const EXECVE:        usize = 221;
    pub const EXIT:          usize = 93;
    pub const EXIT_GROUP:    usize = 94;
    pub const WAIT4:         usize = 260;
    pub const GETPID:        usize = 172;
    pub const GETPPID:       usize = 173;
    pub const CLOCK_GETTIME: usize = 113;
    pub const NANOSLEEP:     usize = 101;
    pub const KILL:          usize = 129;
    pub const GETDENTS64:    usize = 61;
    pub const FSTAT:         usize = 80;
    pub const NEWFSTATAT:    usize = 79;
    pub const FCNTL:         usize = 25;
    pub const DUP:           usize = 23;
    pub const DUP3:          usize = 24;
    pub const PIPE2:         usize = 59;
    pub const GETCWD:        usize = 17;
    pub const CHDIR:         usize = 49;
    pub const MKDIRAT:       usize = 34;
    pub const UNLINKAT:      usize = 35;
    pub const RENAMEAT:      usize = 38;
    pub const SOCKET:        usize = 198;
    pub const BIND:          usize = 200;
    pub const CONNECT:       usize = 203;
    pub const LISTEN:        usize = 201;
    pub const ACCEPT4:       usize = 242;
    pub const SENDTO:        usize = 206;
    pub const RECVFROM:      usize = 207;
    pub const SCHED_YIELD:   usize = 124;
    pub const FUTEX:         usize = 98;
    pub const SET_TID_ADDR:  usize = 96;
    pub const RT_SIGACTION:  usize = 134;
    pub const RT_SIGPROCMASK:usize = 135;
    pub const RT_SIGRETURN:  usize = 139;
    pub const MPROTECT:      usize = 226;
    pub const UMASK:         usize = 166;
    pub const GETUID:        usize = 174;
    pub const GETGID:        usize = 176;
    pub const GETEUID:       usize = 175;
    pub const GETEGID:       usize = 177;
}
