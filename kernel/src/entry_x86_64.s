// x86-64 Limine entry point — kernel/src/entry_x86_64.s
//
// Limine loads the kernel as an ELF and jumps to _start in 64-bit long mode.
// On entry:
//   • CPU is in 64-bit long mode, CPL 0.
//   • Interrupts disabled (RFLAGS.IF = 0).
//   • A flat GDT is loaded: null @ 0x00, 64-bit code @ 0x08, data @ 0x10.
//   • Paging is active; identity map + HHDM are set up by Limine.
//   • RSP is NOT guaranteed — we set it up below.
//   • Boot information is NOT in registers; use boot::limine request structs.
//
// We must:
//   1. Set up a 64-bit stack.
//   2. Zero the BSS.
//   3. Call kernel_main(0)  [arg = 0 signals Limine mode].
//
// Ref: Limine Boot Protocol §entry-point

    .section .text
    .code64
    .global _start
_start:
    cli

    // ── 64-bit stack ──────────────────────────────────────────────────────────
    leaq    __stack_top(%rip), %rsp

    // ── Zero BSS ──────────────────────────────────────────────────────────────
    leaq    __bss_start(%rip), %rdi
    leaq    __bss_end(%rip),   %rcx
    subq    %rdi, %rcx
    shrq    $3,   %rcx          // byte count → u64 count
    xorq    %rax, %rax
    rep stosq

    // ── Call kernel_main(boot_info_addr = 0) ──────────────────────────────────
    // Limine boot info is obtained from static request/response structs
    // (boot::limine), not from a pointer argument.  Pass 0 to distinguish
    // from a multiboot2 address.
    xorl    %edi, %edi
    callq   kernel_main

.Lhalt:
    hlt
    jmp     .Lhalt
