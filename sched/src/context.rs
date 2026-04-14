//! CPU context save/restore — the foundation of context switching.
//!
//! `cpu_switch_to(old, new)` saves callee-saved registers into `*old` and
//! restores them from `*new`, transferring execution to the new task.
//!
//! AArch64: saves x19-x28, x29(fp), x30(lr), SP into `CpuContext`.
//! x86-64:  pushes rbx/rbp/r12-r15 onto the task stack, then stores rsp.

/// Architecture-specific saved context for one schedulable task.
///
/// Only callee-saved state is stored here; the task is responsible for
/// saving caller-saved registers before any blocking call.
#[cfg(target_arch = "aarch64")]
#[repr(C)]
pub struct CpuContext {
    /// x19, x20, x21, x22, x23, x24, x25, x26, x27, x28, x29(fp), x30(lr)
    /// Offsets: 0, 8, 16, 24, 32, 40, 48, 56, 64, 72, 80, 88
    pub gregs: [u64; 12],
    /// SP_EL1 — saved at offset 96.
    pub sp: u64,
}

/// On all non-AArch64 targets (x86-64 and future ports).
#[cfg(not(target_arch = "aarch64"))]
#[repr(C)]
pub struct CpuContext {
    /// Saved kernel stack pointer.
    /// rbx, rbp, r12–r15 are pushed onto the task's stack before rsp is saved.
    pub rsp: u64,
}

impl CpuContext {
    /// A zeroed context, suitable as the initial scheduler idle context.
    pub const fn zeroed() -> Self {
        #[cfg(target_arch = "aarch64")]
        { Self { gregs: [0u64; 12], sp: 0 } }
        #[cfg(not(target_arch = "aarch64"))]
        { Self { rsp: 0 } }
    }

    /// Build a context for a brand-new kernel-mode task.
    ///
    /// On the first `cpu_switch_to` into this context:
    /// - AArch64: `ret` jumps to `entry` (pre-loaded into x30/lr).
    /// - x86-64:  `ret` pops `entry` from the pre-built stack frame.
    pub fn new_task(entry: usize, stack_top: usize) -> Self {
        #[cfg(target_arch = "aarch64")]
        {
            let mut c = Self::zeroed();
            c.gregs[11] = entry as u64; // x30 (lr) = entry point
            c.sp        = stack_top as u64;
            c
        }

        #[cfg(not(target_arch = "aarch64"))]
        {
            // Pre-build a stack frame that cpu_switch_to will pop on first entry.
            // Layout from rsp (low) → stack_top (high):
            //   rsp+0:  rbx = 0   (first pop)
            //   rsp+8:  rbp = 0
            //   rsp+16: r12 = 0
            //   rsp+24: r13 = 0
            //   rsp+32: r14 = 0
            //   rsp+40: r15 = 0
            //   rsp+48: entry     (ret target — popped last)
            let frame = stack_top.wrapping_sub(7 * 8);
            unsafe {
                let p = frame as *mut u64;
                p.add(0).write(0);
                p.add(1).write(0);
                p.add(2).write(0);
                p.add(3).write(0);
                p.add(4).write(0);
                p.add(5).write(0);
                p.add(6).write(entry as u64);
            }
            Self { rsp: frame as u64 }
        }
    }

    /// Build a context for a new user-mode task (AArch64 only).
    ///
    /// When the scheduler first switches to this task, `cpu_switch_to` loads
    /// x30 = `ret_to_user` and branches there via `ret`.  `ret_to_user` then
    /// pops the three words below off the kernel stack and `eret`s to EL0.
    ///
    /// Kernel stack frame layout built here (from `kernel_stack_top - 24`):
    ///   [ksp+0]:  SP_EL0   = user stack pointer
    ///   [ksp+8]:  ELR_EL1  = user entry point
    ///   [ksp+16]: SPSR_EL1 = 0 (EL0t, all interrupts unmasked)
    #[cfg(target_arch = "aarch64")]
    pub fn new_user_task(user_entry: usize, user_sp: usize, kernel_stack_top: usize) -> Self {
        extern "C" { fn ret_to_user(); }
        let frame = kernel_stack_top.wrapping_sub(3 * 8);
        unsafe {
            let p = frame as *mut u64;
            p.add(0).write(user_sp as u64);       // SP_EL0
            p.add(1).write(user_entry as u64);    // ELR_EL1
            p.add(2).write(0u64);                 // SPSR_EL1 = EL0t
        }
        let mut c = Self::zeroed();
        c.gregs[11] = ret_to_user as *const () as u64; // x30 → ret_to_user trampoline
        c.sp        = frame as u64;
        c
    }
}

extern "C" {
    /// Switch CPU from `old` context to `new` context.
    ///
    /// Saves callee-saved registers into `*old` and restores from `*new`.
    /// Returns in the execution context of `new`.
    ///
    /// # Safety
    /// Both pointers must be valid, non-null, and aligned.
    /// `new` must have been initialised by a prior `cpu_switch_to` or
    /// by `CpuContext::new_task`.
    pub fn cpu_switch_to(old: *mut CpuContext, new: *const CpuContext);
}

// ─── AArch64 context switch ────────────────────────────────────────────────

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(r#"
.global cpu_switch_to
.type   cpu_switch_to, %function
cpu_switch_to:
    // x0 = *mut CpuContext (old):   gregs[0..11] @ byte 0..88, sp @ byte 96
    // x1 = *const CpuContext (new)

    // ── save outgoing task ───────────────────────────────────────────────────
    stp  x19, x20, [x0, #0]
    stp  x21, x22, [x0, #16]
    stp  x23, x24, [x0, #32]
    stp  x25, x26, [x0, #48]
    stp  x27, x28, [x0, #64]
    stp  x29, x30, [x0, #80]    // fp (x29) and lr (x30)
    mov  x9,  sp
    str  x9,  [x0, #96]

    // ── restore incoming task ────────────────────────────────────────────────
    ldp  x19, x20, [x1, #0]
    ldp  x21, x22, [x1, #16]
    ldp  x23, x24, [x1, #32]
    ldp  x25, x26, [x1, #48]
    ldp  x27, x28, [x1, #64]
    ldp  x29, x30, [x1, #80]    // x30 = return addr (existing) or entry (new)
    ldr  x9,  [x1, #96]
    mov  sp,  x9

    ret                          // branch to x30
"#);

// ─── x86-64 context switch (AT&T syntax) ──────────────────────────────────

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(r#"
.global cpu_switch_to
.type   cpu_switch_to, @function
cpu_switch_to:
    // rdi = *mut CpuContext (old)   – CpuContext.rsp at offset 0
    // rsi = *const CpuContext (new)

    // Push callee-saved registers onto the outgoing task's kernel stack.
    pushq %rbx
    pushq %rbp
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    movq  %rsp, (%rdi)      // save rsp into old->rsp

    movq  (%rsi), %rsp      // load rsp from new->rsp
    popq  %r15
    popq  %r14
    popq  %r13
    popq  %r12
    popq  %rbp
    popq  %rbx
    retq                    // jump to return address / entry point
"#);
