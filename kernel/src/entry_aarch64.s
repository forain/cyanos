// AArch64 bare-metal entry point — kernel/src/entry_aarch64.s
//
// Supports two boot environments:
//   QEMU -machine virt : enters at EL1 directly.
//   Raspberry Pi 5     : firmware (VideoCore / TF-A) enters at EL2.
//
// On entry (Linux kernel ABI):
//   x0  = physical address of device-tree blob (DTB), or 0
//   x1-x3 = 0 (reserved)
//   SP  = undefined (we set it here)
//   MMU = off, caches = off, interrupts = off
//
// Boot sequence:
//   1. Park secondary CPUs.
//   2. Detect current EL; if EL2, configure HCR_EL2 and ERET to EL1h.
//   3. Set up stack (SP_EL1 after the drop).
//   4. Zero BSS.
//   5. Clear unwanted SCTLR_EL1 bits (MMU/cache off).
//   6. Install VBAR_EL1.
//   7. Call kernel_main(dtb_ptr).
//
// Ref: Linux arch/arm64/kernel/head.S; ARM DDI 0487 §D1.10

.section ".text.boot", "ax", @progbits
.global _start
_start:
    // Debug: _start reached - write 'B' IMMEDIATELY
    mov     x20, 0x09000000         // QEMU virt UART base
    mov     w21, #'B'
    str     w21, [x20]

    // ── Park all secondary CPUs (only Aff0 == 0 proceeds) ────────────────────
    // TEMPORARILY DISABLED to test for CPU parking issues
    // mrs     x1, mpidr_el1
    // and     x1, x1, #0xFF           // Aff0 field
    // cbnz    x1, .Lcpu_park

    // ── Drop from EL2 → EL1h if required (RPi 5 / TF-A boots at EL2) ────────
    //
    // CurrentEL[3:2] encodes the current EL:
    //   0b0100 = EL1,  0b1000 = EL2,  0b1100 = EL3
    //
    mrs     x1, CurrentEL
    lsr     x1, x1, #2
    and     x1, x1, #0x3
    cmp     x1, #2
    bne     .Lel1_entry             // already at EL1 (QEMU)

    // Debug: At EL2, need to drop - write 'C'
    mov     x20, 0x09000000
    mov     w21, #'C'
    str     w21, [x20]

    // Running at EL2.  Minimally configure HCR_EL2 and drop to EL1h.

    // HCR_EL2.RW = 1 (bit 31): EL1 executes as AArch64.
    // All other bits 0: no virtualisation, no TGE, no routing.
    mov     x1, #(1 << 31)
    msr     hcr_el2, x1

    // SPSR_EL2 for return to EL1h with all exceptions (D/A/I/F) masked:
    //   M[3:0] = 0b0101  → EL1h (dedicated SP_EL1)
    //   DAIF   = 0b1111  → bits [9:6] all set → 0x3C0
    //   Combined: 0x3C5
    mov     x1, #0x3C5
    msr     spsr_el2, x1

    // ELR_EL2 = address to return to after ERET.
    adr     x1, .Lel1_entry
    msr     elr_el2, x1
    isb

    eret                            // drops to EL1h, resumes at .Lel1_entry

.Lel1_entry:
    // Debug: Reached EL1 entry - write 'D'
    mov     x20, 0x09000000         // QEMU virt UART base
    mov     w21, #'D'
    str     w21, [x20]

    // ── Set up initial stack (SP_EL1) ─────────────────────────────────────────
    // Use the stack defined in Rust
    adrp    x1, EARLY_STACK
    add     x1, x1, :lo12:EARLY_STACK
    mov     x2, #0x10000            // 64 KiB
    add     x1, x1, x2
    mov     sp, x1

    // Debug: Stack set up - write 'E'
    mov     x20, 0x09000000
    mov     w21, #'E'
    str     w21, [x20]

    // ── Preserve DTB pointer across the BSS clear (x0 is caller-saved) ───────
    mov     x19, x0                 // x19 is callee-saved

    // Debug: Starting BSS clear - write 'F'
    mov     x20, 0x09000000
    mov     w21, #'F'
    str     w21, [x20]

    // ── Zero the BSS section ──────────────────────────────────────────────────
    adrp    x0, __bss_start
    add     x0, x0, :lo12:__bss_start
    adrp    x1, __bss_end
    add     x1, x1, :lo12:__bss_end
    sub     x1, x1, x0
    cbz     x1, .Lbss_done
.Lbss_loop:
    str     xzr, [x0], #8
    subs    x1, x1, #8
    bgt     .Lbss_loop
.Lbss_done:

    // Debug: BSS cleared - write 'G'
    mov     x20, 0x09000000
    mov     w21, #'G'
    str     w21, [x20]

    // ── Minimal EL1 system register setup ────────────────────────────────────
    // Clear SCTLR_EL1: disable MMU (M), data cache (C), instruction cache (I).
    // Leave strict alignment off (A=0) so Rust does not fault before mm::init().
    mrs     x1, sctlr_el1
    mov     x2, #(1 << 0)          // M: MMU enable
    orr     x2, x2, #(1 << 2)      // C: D-cache enable
    orr     x2, x2, #(1 << 12)     // I: I-cache enable
    bic     x1, x1, x2
    msr     sctlr_el1, x1
    isb

    // ── Install exception vector table ────────────────────────────────────────
    // IMPORTANT: adrp alone gives the 4-KiB page BASE containing the label,
    // not the label itself.  __exception_vectors is 2-KiB aligned (not 4-KiB),
    // so it may sit 0x800 bytes into a page.  Must add the page offset with
    // the :lo12: relocation to get the exact address for VBAR_EL1.
    adrp    x1, .Llocal_exception_vectors
    add     x1, x1, :lo12:.Llocal_exception_vectors
    msr     vbar_el1, x1
    isb

    // ── Call kernel_main(dtb_ptr: usize) ─────────────────────────────────────

    // Debug: Write 'A' after basic setup before kernel_main
    mov     x0, 0x09000000          // QEMU virt UART base
    mov     w1, #'A'
    str     w1, [x0]                // Write to data register

    mov     x0, x19                 // restore DTB pointer as first argument
    bl      kernel_main

    // ── kernel_main returned (should never happen — it returns !) ────────────
    b       .Lcpu_park

.Lcpu_park:
    wfe
    b       .Lcpu_park

// ── Exception Vector Table ──────────────────────────────────────────────────
// 2 KiB-aligned table with 16 × 128-byte slots (ARM ARM requirement).
// For now, all exceptions simply halt to avoid crashes during early boot.

.section ".text", "ax", @progbits
.align 11                           // 2^11 = 2048 = 2 KiB alignment
.Llocal_exception_vectors:

// Current EL, SP0
.align 7; b .Lexc_halt             // Synchronous
.align 7; b .Lexc_halt             // IRQ
.align 7; b .Lexc_halt             // FIQ
.align 7; b .Lexc_halt             // SError

// Current EL, SPx
.align 7; b .Lexc_halt             // Synchronous
.align 7; b .Lexc_halt             // IRQ
.align 7; b .Lexc_halt             // FIQ
.align 7; b .Lexc_halt             // SError

// Lower EL, AArch64
.align 7; b .Lexc_halt             // Synchronous
.align 7; b .Lexc_halt             // IRQ
.align 7; b .Lexc_halt             // FIQ
.align 7; b .Lexc_halt             // SError

// Lower EL, AArch32
.align 7; b .Lexc_halt             // Synchronous
.align 7; b .Lexc_halt             // IRQ
.align 7; b .Lexc_halt             // FIQ
.align 7; b .Lexc_halt             // SError

.Lexc_halt:
    // Write 'E' to UART to indicate exception occurred
    mov     x0, 0x09000000          // QEMU virt UART base
    mov     w1, #'E'
    str     w1, [x0]                // Write to data register
.Lexc_park:
    wfe
    b       .Lexc_park
