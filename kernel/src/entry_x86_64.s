// x86-64 entry point — kernel/src/entry_x86_64.s
//
// GRUB (or QEMU -kernel with multiboot2) enters here in 32-bit protected mode:
//   EAX = 0x36d76289  (multiboot2 magic)
//   EBX = physical address of multiboot2 info struct
//   CS  = 32-bit flat code segment, DPL 0
//   SS  = 32-bit flat data segment
//   Interrupts disabled, A20 line enabled, paging OFF, caches OFF
//
// We must:
//   1. Set up temporary identity-mapped page tables (1 GB via 2 MB huge pages)
//   2. Enable PAE (CR4.PAE)
//   3. Enable long mode (EFER.LME)
//   4. Enable paging (CR0.PG) → 64-bit mode activates
//   5. Far-jump into 64-bit code segment
//   6. Set 64-bit stack, zero BSS, call kernel_main
//
// Ref: AMD64 Architecture Programmer's Manual Vol 2 §14.6;
//      Intel SDM Vol 3A §9.8; Linux arch/x86/boot/

# ── Multiboot2 header (must appear within first 32 KB of the file) ────────────
    .section .multiboot2, "a", @progbits
    .align  8
.Lmb2_start:
    .long   0xE85250D6                               # magic
    .long   0                                        # architecture: i386
    .long   (.Lmb2_end - .Lmb2_start)               # header_length
    .long   -(0xE85250D6 + 0 + (.Lmb2_end - .Lmb2_start))  # checksum
    # ── Framebuffer tag (type 5): request a text console ─────────────────────
    .short  5                                        # type: framebuffer
    .short  1                                        # flags: optional
    .long   20                                       # size
    .long   0                                        # width  (0 = no preference)
    .long   0                                        # height
    .long   0                                        # depth  (0 = no preference)
    .align  8
    # ── End tag ──────────────────────────────────────────────────────────────
    .short  0                                        # type 0 = end
    .short  0                                        # flags
    .long   8                                        # size
.Lmb2_end:

# ── 64-bit GDT (used after long mode is active) ───────────────────────────────
    .section .rodata
    .align  8
.Lgdt64:
    .quad   0x0000000000000000   # 0x00: null segment
    .quad   0x00AF9A000000FFFF   # 0x08: 64-bit code, DPL 0 (L=1, P=1, S=1)
    .quad   0x00CF92000000FFFF   # 0x10: 64-bit data, DPL 0
.Lgdt64_end:
    .align  4
.Lgdt64_ptr:
    .short  (.Lgdt64_end - .Lgdt64 - 1)             # limit
    .long   .Lgdt64                                  # base (32-bit is fine <4GB)

# ── Temporary boot page tables (identity-map first 1 GiB, 2 MiB huge pages) ──
# Placed in .data (not BSS) so they are zero-initialised in the ELF file and
# do not require the entry stub to clear them before use.
    .section .data.pgtables, "aw", @progbits
    .align  4096
.Lboot_pml4:    .fill 4096, 1, 0
.Lboot_pdpt:    .fill 4096, 1, 0
.Lboot_pd:      .fill 4096, 1, 0

# ── 32-bit entry ──────────────────────────────────────────────────────────────
    .section .text
    .code32
    .global _start
_start:
    cli
    movl    %ebx, %esi          # save multiboot2 info ptr
    movl    %eax, %edi          # save multiboot2 magic

    # Set up 32-bit stack (in kernel BSS, just past BSS end symbol)
    leal    __stack_top, %esp

    # ── Build page tables ─────────────────────────────────────────────────────
    # PML4[0] = &PDPT | PRESENT | WRITABLE
    leal    .Lboot_pdpt, %eax
    orl     $3, %eax
    movl    %eax, .Lboot_pml4

    # PDPT[0] = &PD | PRESENT | WRITABLE
    leal    .Lboot_pd, %eax
    orl     $3, %eax
    movl    %eax, .Lboot_pdpt

    # PD[0..511]: each entry = i*2MB | PRESENT | WRITABLE | PS (huge page)
    xorl    %ecx, %ecx
.Lpd_loop:
    movl    %ecx, %eax
    shll    $21, %eax           # i * 0x200000 (2 MiB)
    orl     $0x83, %eax         # PRESENT | WRITABLE | PS
    movl    %eax, .Lboot_pd(,%ecx,8)
    movl    $0,   .Lboot_pd+4(,%ecx,8)   # high 32 bits = 0
    incl    %ecx
    cmpl    $512, %ecx
    jne     .Lpd_loop

    # ── Enable PAE ────────────────────────────────────────────────────────────
    leal    .Lboot_pml4, %eax
    movl    %eax, %cr3
    movl    %cr4,  %eax
    orl     $0x20, %eax         # CR4.PAE
    movl    %eax,  %cr4

    # ── Enable Long Mode (LME) in EFER MSR ───────────────────────────────────
    movl    $0xC0000080, %ecx   # EFER MSR number
    rdmsr
    orl     $0x100, %eax        # EFER.LME
    wrmsr

    # ── Enable paging (activates IA-32e / long mode) ─────────────────────────
    movl    %cr0, %eax
    orl     $0x80000001, %eax   # CR0.PE | CR0.PG
    movl    %eax, %cr0

    # ── Load 64-bit GDT and far-jump into code segment 0x08 ──────────────────
    lgdt    .Lgdt64_ptr
    ljmpl   $0x08, $.Lentry64

# ── 64-bit code ───────────────────────────────────────────────────────────────
    .code64
.Lentry64:
    movw    $0x10, %ax
    movw    %ax,   %ds
    movw    %ax,   %es
    movw    %ax,   %ss
    xorw    %ax,   %ax
    movw    %ax,   %fs
    movw    %ax,   %gs

    # 64-bit stack
    leaq    __stack_top(%rip), %rsp

    # ── Zero BSS ──────────────────────────────────────────────────────────────
    leaq    __bss_start(%rip), %rdi
    leaq    __bss_end(%rip),   %rcx
    subq    %rdi, %rcx
    shrq    $3,   %rcx          # byte count → u64 count
    xorq    %rax, %rax
    rep stosq

    # ── Call kernel_main(mbi_ptr: usize) ─────────────────────────────────────
    # System V AMD64 ABI: first arg in %rdi
    movl    %esi, %edi          # zero-extend 32-bit mbi ptr to 64-bit rdi
    callq   kernel_main

.Lhalt:
    hlt
    jmp     .Lhalt
