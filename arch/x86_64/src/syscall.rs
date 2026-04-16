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
//!
//! # Per-CPU stacks and SWAPGS
//!
//! The old implementation used a single global `_syscall_user_rsp` save-slot
//! and a single `_syscall_stack`, both SMP-unsafe.
//!
//! The new implementation uses per-CPU data accessed via the GS segment:
//!   - Each CPU has a `PerCpuSyscall` struct with a private kernel stack top
//!     and a user-RSP save slot.
//!   - `IA32_KERNEL_GS_BASE` MSR is set to point to that struct.
//!   - On SYSCALL entry `swapgs` activates kernel GS (→ per-CPU struct);
//!     on SYSRET `swapgs` restores user GS.
//!
//! `init()` calls `init_per_cpu(0)` for the BSP.
//! `init_ap()` is called by each AP in `smp::sched_ap_entry` after LAPIC init.

const MSR_EFER:         u32 = 0xC000_0080;
const MSR_STAR:         u32 = 0xC000_0081;
const MSR_LSTAR:        u32 = 0xC000_0082;
const MSR_FMASK:        u32 = 0xC000_0084;
const MSR_KERNEL_GSBASE: u32 = 0xC000_0102;

unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx")  msr,
        out("eax") lo,
        out("edx") hi,
        options(nomem, nostack, preserves_flags)
    );
    (hi as u64) << 32 | lo as u64
}

unsafe fn wrmsr(msr: u32, val: u64) {
    core::arch::asm!(
        "wrmsr",
        in("ecx")  msr,
        in("eax")  val as u32,
        in("edx")  (val >> 32) as u32,
        options(nomem, nostack, preserves_flags)
    );
}

// ── Per-CPU SYSCALL data ──────────────────────────────────────────────────────

/// Number of CPUs supported (must match `sched::MAX_CPUS`).
const MAX_CPUS:   usize = 8;
/// Size of each CPU's private SYSCALL kernel stack.
const STACK_SIZE: usize = 16 * 1024; // 16 KiB

/// Per-CPU metadata accessed via GS during the SYSCALL path.
///
/// Field offsets are part of the assembly ABI:
///   offset 0  (`gs:0`)  — `kernel_stack_top`: kernel RSP to load on entry.
///   offset 8  (`gs:8`)  — `user_rsp_save`:    slot for the user RSP.
#[repr(C)]
struct PerCpuSyscall {
    kernel_stack_top: u64,
    user_rsp_save:    u64,
}

/// Static kernel stacks, one per CPU (placed in .bss, zero-initialized).
static mut SYSCALL_STACKS: [[u8; STACK_SIZE]; MAX_CPUS] = [[0u8; STACK_SIZE]; MAX_CPUS];

/// Per-CPU SYSCALL metadata (KERNEL_GS_BASE points here for each CPU).
static mut PER_CPU: [PerCpuSyscall; MAX_CPUS] =
    [const { PerCpuSyscall { kernel_stack_top: 0, user_rsp_save: 0 } }; MAX_CPUS];

/// Initialise the per-CPU SYSCALL state for `cpu_id`.
///
/// Sets `PER_CPU[cpu_id].kernel_stack_top` to the top of the static stack
/// for that CPU, then writes the address of `PER_CPU[cpu_id]` into the
/// `IA32_KERNEL_GS_BASE` MSR so that `swapgs` on SYSCALL entry makes
/// GS point directly to the per-CPU struct.
fn init_per_cpu(cpu_id: usize) {
    let idx = cpu_id.min(MAX_CPUS - 1);
    unsafe {
        // Stack grows downward; top = base + size.
        let stack_top = SYSCALL_STACKS[idx].as_ptr().add(STACK_SIZE) as u64;
        PER_CPU[idx].kernel_stack_top = stack_top;

        let per_cpu_ptr = core::ptr::addr_of!(PER_CPU[idx]) as u64;
        wrmsr(MSR_KERNEL_GSBASE, per_cpu_ptr);
    }
}

/// Configure SYSCALL/SYSRET MSRs and initialise per-CPU state for the BSP (CPU 0).
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

    // BSP is CPU 0.
    init_per_cpu(0);
}

/// Initialise per-CPU SYSCALL state for an Application Processor.
///
/// Must be called after `apic::init()` (so `arch_cpu_id()` returns the
/// correct LAPIC-derived CPU index).  Called from `smp::sched_ap_entry`.
pub fn init_ap() {
    let cpu_id = unsafe { crate::smp::arch_cpu_id() };
    init_per_cpu(cpu_id);
}

// ── SYSCALL entry trampoline ──────────────────────────────────────────────────
//
// Uses per-CPU stacks and save slots accessed through the GS segment.
// FMASK clears IF on SYSCALL so no maskable interrupt can fire between
// swapgs and the callq, preventing GS from being in an inconsistent state.

core::arch::global_asm!(r#"
.global syscall_entry
.type   syscall_entry, @function
syscall_entry:
    // 1. Activate kernel GS (IA32_KERNEL_GS_BASE → GS; user GS stashed).
    swapgs

    // 2. Save user RSP; switch to this CPU's kernel SYSCALL stack.
    movq  %rsp, %gs:8    // PerCpuSyscall.user_rsp_save = user RSP
    movq  %gs:0, %rsp    // RSP = PerCpuSyscall.kernel_stack_top

    // 3. Preserve caller-saved regs clobbered by the C call.
    pushq %r11            // user RFLAGS (written by SYSCALL)
    pushq %rcx            // user RIP    (written by SYSCALL)

    // 4. Rearrange registers for System V C calling convention:
    //    syscall_dispatch(number:rdi, a0:rsi, a1:rdx, a2:rcx)
    //    On entry: rax=number, rdi=a0, rsi=a1, rdx=a2
    movq  %rdx, %rcx    // a2 → rcx  (save before rdx is overwritten)
    movq  %rsi, %rdx    // a1 → rdx
    movq  %rdi, %rsi    // a0 → rsi
    movq  %rax, %rdi    // number → rdi
    callq syscall_dispatch
    // rax = return value (isize) — left in rax for SYSRET.

    // 5. Restore saved regs.
    popq  %rcx            // user RIP   → rcx
    popq  %r11            // user RFLAGS → r11

    // 6. Restore user RSP and deactivate kernel GS.
    movq  %gs:8, %rsp
    swapgs

    // 7. Return to user space.
    sysretq
"#);

extern "C" {
    fn syscall_entry();
}
