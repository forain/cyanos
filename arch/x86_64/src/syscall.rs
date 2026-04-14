//! x86-64 SYSCALL/SYSRET entry point and MSR setup.
//!
//! On SYSCALL (hardware behaviour):
//!   CS  = STAR[47:32] = 0x08 (kernel code)
//!   SS  = STAR[47:32] + 8 = 0x10 (kernel data)
//!   RIP = LSTAR (→ syscall_entry)
//!   RFLAGS &= ~FMASK  (bit 9 cleared → interrupts disabled during syscall)
//!   RCX = user RIP  (restored by SYSRET)
//!   R11 = user RFLAGS (restored by SYSRET)
//!   RSP = unchanged (still user RSP — we switch it manually)
//!
//! Register convention from user space:
//!   RAX = syscall number
//!   RDI = a0, RSI = a1, RDX = a2, R10 = a3, R8 = a4, R9 = a5
//!   (R10 instead of RCX because SYSCALL clobbers RCX)
//!
//! `syscall_dispatch(number, a0, a1, a2)` uses System V C ABI:
//!   RDI = number (from RAX)
//!   RSI = a0     (from RDI)
//!   RDX = a1     (from RSI)
//!   RCX = a2     (from RDX)

const MSR_EFER:  u32 = 0xC000_0080;
const MSR_STAR:  u32 = 0xC000_0081;
const MSR_LSTAR: u32 = 0xC000_0082;
const MSR_FMASK: u32 = 0xC000_0084;

unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx")  msr,
        out("eax") lo,
        out("edx") hi,
        options(nomem, nostack)
    );
    (hi as u64) << 32 | lo as u64
}

unsafe fn wrmsr(msr: u32, val: u64) {
    core::arch::asm!(
        "wrmsr",
        in("ecx")  msr,
        in("eax")  val as u32,
        in("edx")  (val >> 32) as u32,
        options(nomem, nostack)
    );
}

/// Configure SYSCALL/SYSRET MSRs and point the handler at `syscall_entry`.
pub fn init() {
    unsafe {
        // Enable SCE (System Call Extensions) in EFER.
        wrmsr(MSR_EFER, rdmsr(MSR_EFER) | 1);

        // STAR segments:
        //   bits[47:32] = 0x08  → SYSCALL CS = 0x08, SS = 0x10
        //   bits[63:48] = 0x10  → SYSRET64 CS = 0x10+16 = 0x20|3, SS = 0x10+8 = 0x18|3
        wrmsr(MSR_STAR, (0x0008u64 << 32) | (0x0010u64 << 48));

        // LSTAR: entry point for 64-bit SYSCALL.
        wrmsr(MSR_LSTAR, syscall_entry as *const () as u64);

        // FMASK: clear IF (bit 9) on SYSCALL so we run with interrupts disabled.
        wrmsr(MSR_FMASK, 1 << 9);
    }
}

// ── SYSCALL entry trampoline (AT&T syntax) ────────────────────────────────────
//
// Uses a per-kernel static 16 KiB stack (_syscall_stack_bytes / _syscall_stack_top)
// and a single-word save area (_syscall_user_rsp) for the user RSP.
// Safe because FMASK disables interrupts and we are single-CPU.

core::arch::global_asm!(r#"
.global syscall_entry
.type   syscall_entry, @function
syscall_entry:
    // 1. Save user RSP; switch to the kernel SYSCALL stack.
    movq  %rsp, _syscall_user_rsp(%rip)
    leaq  _syscall_stack_top(%rip), %rsp

    // 2. Preserve user RIP (rcx) and user RFLAGS (r11) across the dispatch call.
    pushq %r11
    pushq %rcx

    // 3. Rearrange registers for System V C calling convention:
    //    syscall_dispatch(number:rdi, a0:rsi, a1:rdx, a2:rcx)
    //    On entry: rax=number, rdi=a0, rsi=a1, rdx=a2
    movq  %rdx, %rcx    // a2 → rcx  (save before rdx is overwritten)
    movq  %rsi, %rdx    // a1 → rdx
    movq  %rdi, %rsi    // a0 → rsi
    movq  %rax, %rdi    // number → rdi
    callq syscall_dispatch
    // rax = return value (isize) — left in rax for SYSRET.

    // 4. Restore user RIP and RFLAGS; SYSRET will put them back.
    popq  %rcx          // user RIP   → rcx
    popq  %r11          // user RFLAGS → r11

    // 5. Restore user RSP and return to user space.
    movq  _syscall_user_rsp(%rip), %rsp
    sysretq             // CS=0x23, SS=0x1B, RIP=rcx, RFLAGS=r11

// ── Static data: kernel SYSCALL stack (16 KiB) and user-RSP save slot ───────

.section .bss
.balign 16
_syscall_stack_bytes:
    .skip 16384
_syscall_stack_top:

.section .data
.balign 8
_syscall_user_rsp:
    .quad 0
"#);

extern "C" {
    fn syscall_entry();
}
