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

/// Set the kernel stack top for EL0→EL1 exception entry on the current CPU.
///
/// Stores `kst` in TPIDR_EL1, which the EL0 exception entry stubs reload into
/// SP before saving any registers.  This mirrors x86-64's TSS.rsp0 update and
/// ensures that each user task gets a fresh kernel stack on every exception,
/// regardless of what SP_EL1 happened to be before the EL0→EL1 transition.
///
/// Called from `sched::run()` before every `cpu_switch_to` into a user task.
#[no_mangle]
pub unsafe extern "C" fn arch_set_kernel_stack(kst: u64) {
    core::arch::asm!(
        "msr tpidr_el1, {k}",
        k = in(reg) kst,
        options(nostack)
    );
}

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
    b exc_el0_irq
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

// ── Macro: reload SP_EL1 from TPIDR_EL1 on EL0 entry ────────────────────────
//
// TPIDR_EL1 holds the current task's kernel stack top, written by
// arch_set_kernel_stack() before each cpu_switch_to.
//
// Technique: temporarily stash x9 in sp_el0 (user SP register, which the
// hardware preserves separately and restores on eret), load TPIDR_EL1 into
// x9, move it into sp, then recover x9 from sp_el0.  This keeps the user's
// sp_el0 intact (we restore it before any saves touch the stack).
.macro reload_kernel_sp
    msr  sp_el0, x9           // stash x9; preserves user SP_EL0 value below
    mrs  x9, tpidr_el1        // x9 = kernel stack top (0 if never set)
    cbz  x9, 1f               // skip reload if not set yet (early boot)
    mov  sp, x9               // reset SP to kernel stack top
1:  mrs  x9, sp_el0           // restore x9; user's sp_el0 is back in sp_el0
.endm

// ── EL0-64 IRQ — save caller-saved regs, reload KSP, dispatch, eret ──────────
exc_el0_irq:
    reload_kernel_sp
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

    bl   irq_dispatch

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

// ── EL1h IRQ — save caller-saved regs, dispatch, restore, eret ───────────────
// Does NOT reload the kernel SP (already on the correct EL1 stack).
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
//
// Stack layout after the stp sequence (SP grows down, lowest addr = top):
//   [sp+ 0]: x8   [sp+ 8]: x9
//   [sp+16]: x6   [sp+24]: x7
//   [sp+32]: x4   [sp+40]: x5
//   [sp+48]: x2   [sp+56]: x3
//   [sp+64]: x0   [sp+72]: x1
//   [sp+80]: x29  [sp+88]: x30
exc_el0_sync:
    // Reload SP_EL1 to the task's kernel stack top (via TPIDR_EL1).
    // This ensures a fresh kernel stack frame on every EL0→EL1 entry,
    // regardless of any prior depth on SP_EL1.
    reload_kernel_sp
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

    // SVC: x0-x2 = user args a0/a1/a2, x8 = syscall number — still live.
    // Build syscall_dispatch(number, a0, a1, a2) in x0-x3.
    // Rearrange without clobbering a live source before reading it:
    mov  x3,  x2              // a2 → x3
    mov  x2,  x1              // a1 → x2
    mov  x1,  x0              // a0 → x1
    mov  x0,  x8              // syscall number → x0
    bl   syscall_dispatch     // returns result in x0
    // Store return value into the saved x0 slot so it is restored by eret.
    str  x0,  [sp, #64]
    b    exc_el0_return

exc_el0_fault:
    mrs  x0,  esr_el1
    mrs  x1,  elr_el1
    bl   exc_el0_fault_handler   // panics; falls through to exc_el0_return

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

// ── IRQ dispatch table ────────────────────────────────────────────────────────
//
// Handlers are registered at init time (single-CPU, interrupts disabled) and
// read-only from IRQ context, so no lock is needed.

pub const MAX_IRQS: usize = 1020;

static mut IRQ_HANDLERS: [Option<fn(u32)>; MAX_IRQS] = [None; MAX_IRQS];

/// Register a handler for the given GIC IRQ ID.
///
/// # Safety
/// Must be called before the corresponding IRQ is unmasked (typically during
/// driver init with interrupts disabled).  IRQ context must never call this.
pub unsafe fn register_irq(id: u32, handler: fn(u32)) {
    if (id as usize) < MAX_IRQS {
        IRQ_HANDLERS[id as usize] = Some(handler);
    }
}

// ── Rust-side handlers ────────────────────────────────────────────────────────

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
    } else if (id as usize) < MAX_IRQS {
        if let Some(handler) = IRQ_HANDLERS[id as usize] {
            handler(id);
        }
    }

    super::gic::eoi(iar);

    // After acknowledging the interrupt, check if the scheduler wants to
    // preempt the current task.  We are still in exception context here, but
    // yield_now() saves the task's callee-saved registers via cpu_switch_to
    // and returns normally when the task is resumed; the exc_irq asm epilogue
    // then restores caller-saved registers and issues eret as usual.
    sched::preempt_check();
}

/// EL1 synchronous exception — always a kernel bug; panic with diagnostics.
#[no_mangle]
unsafe extern "C" fn exc_el1_sync_handler(esr: u64, elr: u64) {
    panic!("EL1 sync exception: ESR={:#010x} ELR={:#010x}", esr, elr);
}

/// EL0 fault (non-SVC) — attempt demand-paging, then kill on unhandled faults.
///
/// EC values that indicate a translation or access-flag fault (i.e. "page not
/// present") from EL0:
///   0x20 — Instruction Abort from EL0 (EL0 Inst Abort)
///   0x21 — Instruction Abort from EL0 (EL0 Inst Abort, current EL)  [unused]
///   0x24 — Data Abort from EL0
///   0x25 — Data Abort from EL0 (current EL)                          [unused]
///
/// IFSR/DFSR LSB (ISS[5:0]) == 0b0001xx / 0b0010xx indicate translation
/// faults at levels 1–3.  We delegate all EL0 aborts to the VMM demand-paging
/// path; if it declines (no matching lazy VMA) we kill the task.
#[no_mangle]
unsafe extern "C" fn exc_el0_fault_handler(esr: u64, elr: u64) {
    let ec = (esr >> 26) & 0x3F;  // Exception Class

    // Data Abort (0x24) or Instruction Abort (0x20) from EL0.
    let is_abort = ec == 0x24 || ec == 0x20;

    if is_abort {
        // FAR_EL1 holds the faulting virtual address for aborts.
        let far: u64;
        core::arch::asm!("mrs {}, far_el1", out(reg) far, options(nomem, nostack));

        if sched::handle_page_fault(far as usize) {
            // Fault handled by the demand-paging path — resume the task.
            return;
        }
    }

    // Unhandled fault — print a brief serial diagnostic then kill the task.
    extern "C" { fn arch_serial_putc(b: u8); }
    let msg = b"EL0 fault: task killed\r\n";
    for &b in msg { arch_serial_putc(b); }
    let _ = elr;
    sched::exit(1);
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
