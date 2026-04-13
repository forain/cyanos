// AArch64 bare-metal entry point — kernel/src/entry_aarch64.s
//
// QEMU -machine virt boots an ELF by jumping to its entry point at EL1.
// On entry:
//   x0  = physical address of device-tree blob (DTB)  [Linux kernel ABI]
//   x1-x3 = 0 (reserved by Linux ABI; QEMU sets them 0)
//   SP  = undefined (we set it here)
//   MMU = off, caches = off, interrupts = off
//
// Ref: Linux arch/arm64/kernel/head.S, §4.1 AArch64 booting

.section ".text.boot", "ax", @progbits
.global _start
_start:
    // ── Park all secondary CPUs (only Aff0 == 0 proceeds) ────────────────────
    mrs     x1, mpidr_el1
    and     x1, x1, #0xFF           // Aff0 field
    cbnz    x1, .Lcpu_park

    // ── Set up initial stack ──────────────────────────────────────────────────
    // __stack_top is defined by the linker script (top of a 64 KB block in BSS)
    adrp    x1, __stack_top
    add     x1, x1, :lo12:__stack_top
    mov     sp, x1

    // ── Preserve DTB pointer across the BSS clear (x0 is caller-saved) ───────
    mov     x19, x0                 // x19 is callee-saved

    // ── Zero the BSS section ──────────────────────────────────────────────────
    adrp    x0, __bss_start
    add     x0, x0, :lo12:__bss_start
    adrp    x1, __bss_end
    add     x1, x1, :lo12:__bss_end
    b       .Lbss_check
.Lbss_loop:
    str     xzr, [x0], #8
.Lbss_check:
    cmp     x0, x1
    b.lo    .Lbss_loop

    // ── Minimal EL1 system register setup ────────────────────────────────────
    // SCTLR_EL1: disable MMU (M), data cache (C), instruction cache (I).
    // We keep strict alignment off (A=0) so the Rust runtime isn't tripped up
    // before mm::init() aligns everything properly.
    mrs     x1, sctlr_el1
    mov     x2, #(1 << 0)          // M: MMU enable
    orr     x2, x2, #(1 << 2)      // C: D-cache enable
    orr     x2, x2, #(1 << 12)     // I: I-cache enable
    bic     x1, x1, x2
    msr     sctlr_el1, x1
    isb

    // ── Set VBAR_EL1 to our exception vector table ────────────────────────────
    adrp    x1, __exception_vectors
    msr     vbar_el1, x1
    isb

    // ── Call kernel_main(dtb_ptr: usize) ─────────────────────────────────────
    mov     x0, x19                 // restore DTB pointer as first argument
    bl      kernel_main

    // ── kernel_main returned (should never happen — it's -> !) ───────────────
    b       .Lcpu_park

.Lcpu_park:
    wfe
    b       .Lcpu_park
