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
///
/// FPU/SIMD state is always saved eagerly on every context switch.
/// This is simpler than lazy-FPU (trap-on-use) and correct for all workloads.
#[cfg(target_arch = "aarch64")]
#[repr(C)]
pub struct CpuContext {
    /// x19, x20, x21, x22, x23, x24, x25, x26, x27, x28, x29(fp), x30(lr)
    /// Offsets: 0..96 (12 × 8 bytes)
    pub gregs: [u64; 12],
    /// SP_EL1 — offset 96.
    pub sp: u64,
    /// Padding to 16-byte-align fpregs for `stp q` instructions — offset 104.
    pub _pad: u64,
    // ── AArch64 FP/SIMD (FEAT_FP + FEAT_AdvSIMD, mandatory from ARMv8.0) ────
    /// SIMD/FP registers Q0-Q31, each 128 bits wide.
    /// Offset: 112.  Total: 32 × 16 = 512 bytes.
    pub fpregs: [u128; 32],
    /// FPCR (floating-point control register) — offset 624.
    pub fpcr: u64,
    /// FPSR (floating-point status register) — offset 632.
    pub fpsr: u64,
    /// TPIDR_EL0 — user-space thread-pointer register — offset 640.
    /// Used by musl/pthreads as the TLS base pointer.
    pub tpidr_el0: u64,
    /// Padding to maintain 8-byte struct alignment — offset 648.
    pub _pad2: u64,
}

/// On all non-AArch64 targets (x86-64 and future ports).
#[cfg(not(target_arch = "aarch64"))]
#[repr(C)]
pub struct CpuContext {
    /// Saved kernel stack pointer.
    /// rbx, rbp, r12–r15 are pushed onto the task's stack before rsp is saved.
    /// Offset: 0.
    pub rsp: u64,
    // ── x86-64 SSE/AVX state ─────────────────────────────────────────────────
    /// XMM0-XMM15 (128-bit each).  Offset: 8.  Total: 16 × 16 = 256 bytes.
    pub xmm: [u128; 16],
    /// MXCSR (SSE control/status).  Offset: 264.
    pub mxcsr: u32,
    /// Padding — offset 268.
    pub _pad: u32,
    /// FS.base — thread-local storage pointer for musl/pthreads — offset 272.
    /// Saved/restored via RDMSR/WRMSR on MSR_FS_BASE (0xC000_0100).
    pub fs_base: u64,
}

impl CpuContext {
    /// A zeroed context, suitable as the initial scheduler idle context.
    pub const fn zeroed() -> Self {
        #[cfg(target_arch = "aarch64")]
        { Self {
            gregs: [0u64; 12], sp: 0, _pad: 0,
            fpregs: [0u128; 32], fpcr: 0, fpsr: 0,
            tpidr_el0: 0, _pad2: 0,
        } }
        #[cfg(not(target_arch = "aarch64"))]
        // mxcsr 0x1F80 = default SSE control: all exceptions masked, round-to-nearest
        { Self { rsp: 0, xmm: [0u128; 16], mxcsr: 0x1F80, _pad: 0, fs_base: 0 } }
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
            c.gregs[11] = entry as u64;  // x30 (lr) = entry point
            c.sp = stack_top as u64;
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
            let mut c = Self::zeroed();
            c.rsp = frame as u64;
            c
        }
    }

    /// Build a context for a new user-mode task.
    ///
    /// **AArch64**: `cpu_switch_to` loads x30 = `ret_to_user` and branches there
    /// via `ret`.  `ret_to_user` pops SP_EL0/ELR_EL1/SPSR_EL1 and `eret`s to EL0.
    ///
    /// **x86-64**: `cpu_switch_to` pops callee-saved regs, then `ret`s to
    /// `iret_to_user`, which executes `iretq` into the IRET frame below it.
    ///
    /// AArch64 kernel stack layout (from `kernel_stack_top - 24`):
    ///   [ksp+0]:  SP_EL0   = user stack pointer
    ///   [ksp+8]:  ELR_EL1  = user entry point
    ///   [ksp+16]: SPSR_EL1 = 0 (EL0t, all interrupts unmasked)
    ///
    /// x86-64 kernel stack layout (from `kernel_stack_top - 96`):
    ///   [ksp+0..40]: callee-saved regs = 0 (rbx, rbp, r12-r15)
    ///   [ksp+48]:    iret_to_user (ret target)
    ///   [ksp+56]:    user RIP
    ///   [ksp+64]:    user CS  = 0x23
    ///   [ksp+72]:    user RFLAGS = 0x202 (IF set)
    ///   [ksp+80]:    user RSP
    ///   [ksp+88]:    user SS  = 0x1B
    pub fn new_user_task(user_entry: usize, user_sp: usize, kernel_stack_top: usize) -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            extern "C" { fn iret_to_user(); }
            // Frame is 12 words (96 bytes) below stack top.
            // Layout: 6 × callee-saved zeros | iret_to_user | IRET frame (5 words)
            let frame = kernel_stack_top.wrapping_sub(12 * 8);
            unsafe {
                let p = frame as *mut u64;
                p.add(0).write(0);                      // rbx
                p.add(1).write(0);                      // rbp
                p.add(2).write(0);                      // r12
                p.add(3).write(0);                      // r13
                p.add(4).write(0);                      // r14
                p.add(5).write(0);                      // r15
                p.add(6).write(iret_to_user as u64);    // ret target → iretq
                p.add(7).write(user_entry as u64);      // IRET: user RIP
                p.add(8).write(0x23);                   // IRET: user CS  (DPL 3, 64-bit)
                p.add(9).write(0x202);                  // IRET: RFLAGS (IF=1)
                p.add(10).write(user_sp as u64);        // IRET: user RSP
                p.add(11).write(0x1B);                  // IRET: user SS  (DPL 3)
            }
            let mut c = Self::zeroed();
            c.rsp = frame as u64;
            c
        }

        #[cfg(target_arch = "aarch64")]
        {
            extern "C" { fn ret_to_user(); }
            let frame = kernel_stack_top.wrapping_sub(3 * 8);
            unsafe {
                let p = frame as *mut u64;
                p.add(0).write(user_sp as u64);     // SP_EL0
                p.add(1).write(user_entry as u64);  // ELR_EL1
                p.add(2).write(0u64);               // SPSR_EL1 = EL0t
            }
            let mut c = Self::zeroed();
            c.gregs[11] = ret_to_user as *const () as u64;
            c.sp        = frame as u64;
            c
        }
    }
}

/// Full user-register frame saved by the AArch64 EL0 synchronous exception
/// handler at the top of the kernel stack on every EL0→EL1 transition.
///
/// Layout matches the `sub sp, sp, #272` frame in `exc_el0_sync`:
///
/// | Offset | Field      | Description                         |
/// |--------|------------|-------------------------------------|
/// |   0    | x[0..=30]  | General-purpose registers x0–x30    |
/// |  248   | sp_el0     | User stack pointer at exception entry|
/// |  256   | elr_el1    | User PC (return address after SVC)  |
/// |  264   | spsr_el1   | User PSTATE                         |
///
/// Total size: 272 bytes (17 × 16 — 16-byte aligned).
#[cfg(target_arch = "aarch64")]
#[repr(C)]
pub struct UserFrame {
    /// General-purpose registers x0–x30 (31 × 8 = 248 bytes).
    pub x:        [u64; 31],
    /// User stack pointer saved by the EL0 exception stub.
    pub sp_el0:   u64,
    /// ELR_EL1: user-space return address (instruction after SVC).
    pub elr_el1:  u64,
    /// SPSR_EL1: saved user PSTATE.
    pub spsr_el1: u64,
}

#[cfg(target_arch = "aarch64")]
impl UserFrame {
    /// Byte size of the frame — must match the `sub sp, sp, #272` in asm.
    pub const SIZE: usize = core::mem::size_of::<Self>();
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
    // x0 = *mut CpuContext (old)
    // x1 = *const CpuContext (new)
    //
    // CpuContext layout (AArch64):
    //   Bytes   0.. 88: gregs[0..11] (x19-x30, 12 × u64)
    //   Byte       96:  sp            (u64)
    //   Byte      104:  _pad          (u64, alignment padding)
    //   Bytes 112..623: fpregs[0..31] (Q0-Q31, 32 × u128 = 512 bytes, 16-byte aligned)
    //   Byte      624:  fpcr          (u64)
    //   Byte      632:  fpsr          (u64)

    // ── save outgoing integer registers ─────────────────────────────────────
    stp  x19, x20, [x0, #0]
    stp  x21, x22, [x0, #16]
    stp  x23, x24, [x0, #32]
    stp  x25, x26, [x0, #48]
    stp  x27, x28, [x0, #64]
    stp  x29, x30, [x0, #80]    // fp (x29) and lr (x30)
    mov  x9,  sp
    str  x9,  [x0, #96]

    // ── save outgoing FP/SIMD registers ─────────────────────────────────────
    add  x9, x0, #112            // x9 → fpregs[0] (16-byte aligned)
    stp  q0,  q1,  [x9, #0]
    stp  q2,  q3,  [x9, #32]
    stp  q4,  q5,  [x9, #64]
    stp  q6,  q7,  [x9, #96]
    stp  q8,  q9,  [x9, #128]
    stp  q10, q11, [x9, #160]
    stp  q12, q13, [x9, #192]
    stp  q14, q15, [x9, #224]
    stp  q16, q17, [x9, #256]
    stp  q18, q19, [x9, #288]
    stp  q20, q21, [x9, #320]
    stp  q22, q23, [x9, #352]
    stp  q24, q25, [x9, #384]
    stp  q26, q27, [x9, #416]
    stp  q28, q29, [x9, #448]
    stp  q30, q31, [x9, #480]
    mrs  x10, fpcr
    str  x10, [x0, #624]
    mrs  x10, fpsr
    str  x10, [x0, #632]
    // Save user-space TLS pointer (TPIDR_EL0) — offset 640.
    mrs  x10, tpidr_el0
    str  x10, [x0, #640]

    // ── restore incoming integer registers ───────────────────────────────────
    ldp  x19, x20, [x1, #0]
    ldp  x21, x22, [x1, #16]
    ldp  x23, x24, [x1, #32]
    ldp  x25, x26, [x1, #48]
    ldp  x27, x28, [x1, #64]
    ldp  x29, x30, [x1, #80]    // x30 = return addr or entry point
    ldr  x9,  [x1, #96]
    mov  sp,  x9

    // ── restore incoming FP/SIMD registers ───────────────────────────────────
    add  x9, x1, #112            // x9 → fpregs[0] (16-byte aligned)
    ldp  q0,  q1,  [x9, #0]
    ldp  q2,  q3,  [x9, #32]
    ldp  q4,  q5,  [x9, #64]
    ldp  q6,  q7,  [x9, #96]
    ldp  q8,  q9,  [x9, #128]
    ldp  q10, q11, [x9, #160]
    ldp  q12, q13, [x9, #192]
    ldp  q14, q15, [x9, #224]
    ldp  q16, q17, [x9, #256]
    ldp  q18, q19, [x9, #288]
    ldp  q20, q21, [x9, #320]
    ldp  q22, q23, [x9, #352]
    ldp  q24, q25, [x9, #384]
    ldp  q26, q27, [x9, #416]
    ldp  q28, q29, [x9, #448]
    ldp  q30, q31, [x9, #480]
    ldr  x10, [x1, #624]
    msr  fpcr, x10
    ldr  x10, [x1, #632]
    msr  fpsr, x10
    // Restore user-space TLS pointer (TPIDR_EL0) — offset 640.
    ldr  x10, [x1, #640]
    msr  tpidr_el0, x10

    ret                          // branch to x30
"#);

// ─── x86-64 context switch + user-mode trampoline (AT&T syntax) ───────────────

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(r#"
.global cpu_switch_to
.type   cpu_switch_to, @function
cpu_switch_to:
    // rdi = *mut CpuContext (old)
    // rsi = *const CpuContext (new)
    //
    // CpuContext layout (x86-64):
    //   Offset   0: rsp    (u64)
    //   Offset   8: xmm[0..15] (16 × u128 = 256 bytes)
    //   Offset 264: mxcsr  (u32)
    //   Offset 268: _pad   (u32)

    // ── save FS.base (TLS pointer) via RDMSR on MSR_FS_BASE (0xC0000100) ─────
    // RDMSR: ecx=MSR, returns edx:eax.  Combine into rax then store at offset 272.
    movl  $0xC0000100, %ecx
    rdmsr
    shlq  $32, %rdx
    orq   %rdx, %rax
    movq  %rax, 272(%rdi)

    // ── save outgoing SSE/FPU state ──────────────────────────────────────────
    movdqu %xmm0,  8(%rdi)
    movdqu %xmm1,  24(%rdi)
    movdqu %xmm2,  40(%rdi)
    movdqu %xmm3,  56(%rdi)
    movdqu %xmm4,  72(%rdi)
    movdqu %xmm5,  88(%rdi)
    movdqu %xmm6,  104(%rdi)
    movdqu %xmm7,  120(%rdi)
    movdqu %xmm8,  136(%rdi)
    movdqu %xmm9,  152(%rdi)
    movdqu %xmm10, 168(%rdi)
    movdqu %xmm11, 184(%rdi)
    movdqu %xmm12, 200(%rdi)
    movdqu %xmm13, 216(%rdi)
    movdqu %xmm14, 232(%rdi)
    movdqu %xmm15, 248(%rdi)
    stmxcsr 264(%rdi)

    // ── save outgoing integer registers ──────────────────────────────────────
    pushq %rbx
    pushq %rbp
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    movq  %rsp, (%rdi)      // save rsp into old->rsp

    // ── restore incoming integer registers ───────────────────────────────────
    movq  (%rsi), %rsp      // load rsp from new->rsp
    popq  %r15
    popq  %r14
    popq  %r13
    popq  %r12
    popq  %rbp
    popq  %rbx

    // ── restore incoming SSE/FPU state ───────────────────────────────────────
    ldmxcsr 264(%rsi)
    movdqu  8(%rsi),   %xmm0
    movdqu  24(%rsi),  %xmm1
    movdqu  40(%rsi),  %xmm2
    movdqu  56(%rsi),  %xmm3
    movdqu  72(%rsi),  %xmm4
    movdqu  88(%rsi),  %xmm5
    movdqu  104(%rsi), %xmm6
    movdqu  120(%rsi), %xmm7
    movdqu  136(%rsi), %xmm8
    movdqu  152(%rsi), %xmm9
    movdqu  168(%rsi), %xmm10
    movdqu  184(%rsi), %xmm11
    movdqu  200(%rsi), %xmm12
    movdqu  216(%rsi), %xmm13
    movdqu  232(%rsi), %xmm14
    movdqu  248(%rsi), %xmm15

    // ── restore FS.base (TLS pointer) via WRMSR on MSR_FS_BASE ──────────────
    // WRMSR: ecx=MSR, edx:eax=value.  Load from offset 272.
    movq  272(%rsi), %rax
    movq  %rax, %rdx
    shrq  $32, %rdx
    movl  $0xC0000100, %ecx
    wrmsr

    retq                    // jump to return address / entry point

// ── iret_to_user — first entry into a user-space task (x86-64) ───────────────
//
// Called via `retq` from cpu_switch_to when the kernel stack was built by
// CpuContext::new_user_task.  On entry RSP points at the 5-word IRET frame:
//   [rsp+0]:  user RIP
//   [rsp+8]:  user CS  (0x23)
//   [rsp+16]: user RFLAGS (0x202)
//   [rsp+24]: user RSP
//   [rsp+32]: user SS   (0x1b)
.global iret_to_user
.type   iret_to_user, @function
iret_to_user:
    iretq
"#);

