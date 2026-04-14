//! AArch64 exception vector table and handlers.
//!
//! The table must be 2 KiB aligned (VBAR_EL1 requirement).
//! Each of the 16 vector slots is 128 bytes; we branch to out-of-line
//! handlers so the slots only hold a single `b` each.
//!
//! Slots wired up:
//!   EL1h Sync  (0x200) — kernel fault → panic with ESR/ELR
//!   EL1h IRQ   (0x280) — device/timer interrupt → irq_dispatch
//!   EL0-64 Sync (0x400) — SVC #0 → syscall_dispatch
//!   Everything else    → exc_unexpected (panic)
//!
//! Also provides `ret_to_user`: the trampoline used by the scheduler when
//! entering a user-space task for the first time (or after a syscall).

/// Install VBAR_EL1 pointing at our vector table.
pub fn init() {
    unsafe {
        core::arch::asm!(
            "adr x0, __exception_vectors",
            "msr VBAR_EL1, x0",
            "isb",
            options(nostack)
        );
    }
}

// ── Vector table (2 KiB aligned) ──────────────────────────────────────────

core::arch::global_asm!(r#"
.section .text
.balign 2048
.global __exception_vectors
__exception_vectors:
    // EL1t Sync  (SP_EL0) — 0x000
    b exc_el1_sync
    .balign 128
    // EL1t IRQ
    b exc_irq
    .balign 128
    // EL1t FIQ
    b exc_unexpected
    .balign 128
    // EL1t SError
    b exc_unexpected
    .balign 128

    // EL1h Sync  (SP_EL1) — 0x200
    b exc_el1_sync
    .balign 128
    // EL1h IRQ   — 0x280
    b exc_irq
    .balign 128
    // EL1h FIQ
    b exc_unexpected
    .balign 128
    // EL1h SError
    b exc_unexpected
    .balign 128

    // EL0-64 Sync — 0x400
    b exc_el0_sync
    .balign 128
    // EL0-64 IRQ
    b exc_irq
    .balign 128
    // EL0-64 FIQ
    b exc_unexpected
    .balign 128
    // EL0-64 SError
    b exc_unexpected
    .balign 128

    // EL0-32 (AArch32) — 0x600 — not supported
    b exc_unexpected
    .balign 128
    b exc_unexpected
    .balign 128
    b exc_unexpected
    .balign 128
    b exc_unexpected
    .balign 128

// ── IRQ handler — save caller-saved regs, dispatch, restore, eret ─────────
// Handles both EL1h and EL0-64 IRQ vectors.
exc_irq:
    // Save all caller-saved registers (x0-x17, x29=fp, x30=lr).
    stp  x29, x30, [sp, #-16]!
    stp  x0,  x1,  [sp, #-16]!
    stp  x2,  x3,  [sp, #-16]!
    stp  x4,  x5,  [sp, #-16]!
    stp  x6,  x7,  [sp, #-16]!
    stp  x8,  x9,  [sp, #-16]!
    stp  x10, x11, [sp, #-16]!
    stp  x12, x13, [sp, #-16]!
    stp  x14, x15, [sp, #-16]!
    stp  x16, x17, [sp, #-16]!

    bl   irq_dispatch           // Rust handler; may call sched::timer_tick_irq

    ldp  x16, x17, [sp], #16
    ldp  x14, x15, [sp], #16
    ldp  x12, x13, [sp], #16
    ldp  x10, x11, [sp], #16
    ldp  x8,  x9,  [sp], #16
    ldp  x6,  x7,  [sp], #16
    ldp  x4,  x5,  [sp], #16
    ldp  x2,  x3,  [sp], #16
    ldp  x0,  x1,  [sp], #16
    ldp  x29, x30, [sp], #16
    eret

// ── EL1h synchronous exception (kernel fault) ─────────────────────────────
exc_el1_sync:
    mrs  x0, esr_el1
    mrs  x1, elr_el1
    bl   exc_el1_sync_handler   // panics

// ── EL0-64 synchronous exception (SVC / user fault) ───────────────────────
exc_el0_sync:
    // Save caller-saved GPRs onto the kernel stack (SP_EL1 is used here).
    stp  x29, x30, [sp, #-16]!
    stp  x0,  x1,  [sp, #-16]!
    stp  x2,  x3,  [sp, #-16]!
    stp  x4,  x5,  [sp, #-16]!
    stp  x6,  x7,  [sp, #-16]!
    stp  x8,  x9,  [sp, #-16]!

    // Check EC field (ESR_EL1[31:26]). EC 0x15 = SVC AArch64.
    mrs  x9,  esr_el1
    lsr  x9,  x9,  #26
    cmp  x9,  #0x15
    b.ne exc_el0_fault

    // SVC: syscall number in x8, args in x0-x5.
    // Our dispatch(number, a0, a1, a2) uses System V ABI (rdi/rsi/rdx/rcx).
    // Restore a0-a2 from the stack for correct values.
    ldp  x8,  x9,  [sp, #0]      // restore x8 (syscall number), x9 (scratch)
    ldp  x0,  x1,  [sp, #16]     // restore x0 (a0), x1 (a1)
    ldp  x2,  x3,  [sp, #32]     // restore x2 (a2), x3
    // syscall_entry_aarch64(a0=x0, a1=x1, a2=x2, ..., number=x8) → x0
    bl   syscall_entry_aarch64
    // Store return value into the saved x0 slot so it's restored below.
    str  x0,  [sp, #16]
    b    exc_el0_return

exc_el0_fault:
    mrs  x0,  esr_el1
    mrs  x1,  elr_el1
    bl   exc_el0_fault_handler   // panics

exc_el0_return:
    ldp  x8,  x9,  [sp], #16
    ldp  x6,  x7,  [sp], #16
    ldp  x4,  x5,  [sp], #16
    ldp  x2,  x3,  [sp], #16
    ldp  x0,  x1,  [sp], #16
    ldp  x29, x30, [sp], #16
    eret

// ── Unexpected exception ──────────────────────────────────────────────────
exc_unexpected:
    mrs  x0, esr_el1
    mrs  x1, elr_el1
    bl   exc_unexpected_handler  // panics

// ── ret_to_user — first entry into a user-space task ──────────────────────
//
// The scheduler's cpu_switch_to stores ret_to_user as x30 (lr) in the task
// context, so the `ret` at the end of cpu_switch_to jumps here.
//
// On entry the kernel stack contains (built by CpuContext::new_user_task):
//   [sp+0]:  SP_EL0   (user stack pointer)
//   [sp+8]:  ELR_EL1  (user entry point)
//   [sp+16]: SPSR_EL1 (0x00000000 = EL0t, all interrupts unmasked)
.global ret_to_user
.type   ret_to_user, %function
ret_to_user:
    ldr  x0, [sp], #8
    msr  sp_el0,   x0       // user stack pointer
    ldr  x0, [sp], #8
    msr  elr_el1,  x0       // user entry point
    ldr  x0, [sp], #8
    msr  spsr_el1, x0       // saved program status (EL0t)
    dsb  sy
    isb
    mov  x0, #0             // clear x0 (first user argument)
    eret                    // switch to EL0 at ELR_EL1
"#);

// ── Rust-side handlers ────────────────────────────────────────────────────

/// Dispatch an IRQ: acknowledge via GIC, route to the correct handler, EOI.
///
/// Called from `exc_irq` with caller-saved registers already stacked.
/// Must NOT acquire any spin locks (those could be held by interrupted code).
#[no_mangle]
unsafe extern "C" fn irq_dispatch() {
    let iar = super::gic::ack();
    let id  = super::gic::irq_id(iar);
    if id == super::gic::SPURIOUS { return; }

    if id == 30 {
        // PPI #30 = EL1 physical timer.
        super::timer::on_tick();
    }
    // Other IRQs: TODO — route to a device driver table.

    super::gic::eoi(iar);
}

/// EL1 synchronous exception — always a kernel bug; panic with diagnostics.
#[no_mangle]
unsafe extern "C" fn exc_el1_sync_handler(esr: u64, elr: u64) {
    panic!("EL1 sync exception: ESR={:#010x} ELR={:#010x}", esr, elr);
}

/// EL0 fault (non-SVC) — unhandled user fault; panic for now.
#[no_mangle]
unsafe extern "C" fn exc_el0_fault_handler(esr: u64, elr: u64) {
    panic!("EL0 fault: ESR={:#010x} ELR={:#010x}", esr, elr);
}

/// Unexpected vector — should never fire; panic with diagnostics.
#[no_mangle]
unsafe extern "C" fn exc_unexpected_handler(esr: u64, elr: u64) {
    panic!("unexpected exception: ESR={:#010x} ELR={:#010x}", esr, elr);
}

/// AArch64 syscall entry — called from `exc_el0_sync` after EC check.
///
/// Register mapping on entry: x0=a0, x1=a1, x2=a2, x8=syscall_number.
/// Calls `syscall_dispatch` (defined in the `kernel` crate as `#[no_mangle]`).
#[no_mangle]
pub unsafe extern "C" fn syscall_entry_aarch64(
    a0: usize, a1: usize, a2: usize, _a3: usize,
    _a4: usize, _a5: usize, _a6: usize, _a7: usize,
    number: usize,
) -> isize {
    extern "C" {
        fn syscall_dispatch(number: usize, a0: usize, a1: usize, a2: usize) -> isize;
    }
    syscall_dispatch(number, a0, a1, a2)
}
