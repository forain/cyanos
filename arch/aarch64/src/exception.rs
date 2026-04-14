//! AArch64 exception vector table and handler stubs.
//!
//! The table lives at a 2 KiB-aligned address pointed to by VBAR_EL1.
//! Each of the 16 slots is 128 bytes (32 instructions).  We branch to
//! out-of-line handlers so the slots themselves just hold one branch each.
//!
//! Slots implemented:
//!   EL1h Sync  (0x200) — kernel fault → panic with ESR / ELR info
//!   EL0 64 Sync (0x400) — SVC #0 from user space → syscall dispatch
//!   Everything else    — branch to `exc_unexpected` which panics

/// Install VBAR_EL1.  Called once from `arch_aarch64::init()`.
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

// ── Exception vector table (must be 2 KiB aligned) ────────────────────────
core::arch::global_asm!(r#"
.section .text
.balign 2048
.global __exception_vectors
__exception_vectors:
    // EL1t  Sync  (SP_EL0) — offset 0x000
    b exc_el1_sync
    .balign 128
    // EL1t  IRQ
    b exc_unexpected
    .balign 128
    // EL1t  FIQ
    b exc_unexpected
    .balign 128
    // EL1t  SError
    b exc_unexpected
    .balign 128

    // EL1h  Sync  (SP_EL1) — offset 0x200 — kernel-mode exception
    b exc_el1_sync
    .balign 128
    // EL1h  IRQ
    b exc_irq
    .balign 128
    // EL1h  FIQ
    b exc_unexpected
    .balign 128
    // EL1h  SError
    b exc_unexpected
    .balign 128

    // EL0 64-bit Sync — offset 0x400 — syscall / user fault
    b exc_el0_sync
    .balign 128
    // EL0 64-bit IRQ
    b exc_irq
    .balign 128
    // EL0 64-bit FIQ
    b exc_unexpected
    .balign 128
    // EL0 64-bit SError
    b exc_unexpected
    .balign 128

    // EL0 32-bit (AArch32) — offset 0x600 — not supported
    b exc_unexpected
    .balign 128
    b exc_unexpected
    .balign 128
    b exc_unexpected
    .balign 128
    b exc_unexpected
    .balign 128

// ── EL1 synchronous exception (kernel bug / fault) ────────────────────────
exc_el1_sync:
    // Pass ESR_EL1 and ELR_EL1 to the Rust handler for a useful panic.
    mrs  x0, esr_el1
    mrs  x1, elr_el1
    bl   exc_el1_sync_handler
    // Not reached (handler panics).

// ── EL0 64-bit synchronous exception (SVC → syscall) ─────────────────────
exc_el0_sync:
    // Save caller-saved registers; we are now on the kernel stack (SP_EL1).
    stp  x29, x30, [sp, #-16]!
    stp  x0,  x1,  [sp, #-16]!
    stp  x2,  x3,  [sp, #-16]!
    stp  x4,  x5,  [sp, #-16]!
    stp  x6,  x7,  [sp, #-16]!
    stp  x8,  x9,  [sp, #-16]!

    // Check EC field of ESR_EL1: bits[31:26].  EC = 0x15 → SVC AArch64.
    mrs  x9,  esr_el1
    lsr  x9,  x9,  #26
    cmp  x9,  #0x15
    b.ne exc_el0_not_svc

    // SVC: syscall number in x8, args in x0-x5.
    // Reload args (they were stacked above).
    ldp  x6,  x7,  [sp, #32]  // x6, x7 still in regs
    ldp  x4,  x5,  [sp, #48]
    ldp  x2,  x3,  [sp, #64]
    ldp  x0,  x1,  [sp, #80]
    // syscall_dispatch(number=x8, a0=x0, a1=x1, a2=x2) → x0 (return value)
    bl   syscall_entry_aarch64

    // Store return value back to user's x0 slot on the stack.
    str  x0, [sp, #80]
    b    exc_el0_return

exc_el0_not_svc:
    // Other EL0 fault — pass ESR and ELR to Rust handler (panics).
    mrs  x0, esr_el1
    mrs  x1, elr_el1
    bl   exc_el0_fault_handler

exc_el0_return:
    ldp  x8,  x9,  [sp], #16
    ldp  x6,  x7,  [sp], #16
    ldp  x4,  x5,  [sp], #16
    ldp  x2,  x3,  [sp], #16
    ldp  x0,  x1,  [sp], #16
    ldp  x29, x30, [sp], #16
    eret

// ── IRQ stub (timer, devices) ─────────────────────────────────────────────
exc_irq:
    // TODO: save state, call irq_handler, restore, eret.
    b   exc_unexpected

// ── Unexpected exception ──────────────────────────────────────────────────
exc_unexpected:
    mrs  x0, esr_el1
    mrs  x1, elr_el1
    bl   exc_unexpected_handler
"#);

// ── Rust-side handlers ────────────────────────────────────────────────────

#[no_mangle]
unsafe extern "C" fn exc_el1_sync_handler(esr: u64, elr: u64) {
    let _ = (esr, elr);
    panic!("EL1 synchronous exception: ESR={:#010x} ELR={:#010x}", esr, elr);
}

#[no_mangle]
unsafe extern "C" fn exc_el0_fault_handler(esr: u64, elr: u64) {
    panic!("EL0 fault: ESR={:#010x} ELR={:#010x}", esr, elr);
}

#[no_mangle]
unsafe extern "C" fn exc_unexpected_handler(esr: u64, elr: u64) {
    panic!("unexpected exception: ESR={:#010x} ELR={:#010x}", esr, elr);
}

/// AArch64 syscall entry — called from the EL0 sync vector with x8=number,
/// x0-x2 as first three arguments.
///
/// # Safety
/// Called from assembly with caller-saved registers already stacked.
#[no_mangle]
pub unsafe extern "C" fn syscall_entry_aarch64(
    a0: usize, a1: usize, a2: usize, _a3: usize,
    _a4: usize, _a5: usize, _a6: usize, _a7: usize,
    number: usize,
) -> isize {
    // Dispatch to the kernel syscall table (defined in kernel/src/syscall.rs).
    // We call through a weak extern so this crate does not depend on `kernel`.
    extern "C" {
        fn syscall_dispatch(number: usize, a0: usize, a1: usize, a2: usize) -> isize;
    }
    syscall_dispatch(number, a0, a1, a2)
}
