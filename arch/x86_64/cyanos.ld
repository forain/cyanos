/* x86-64 linker script — multiboot2 kernel loaded by GRUB or QEMU -kernel
 *
 * The kernel is linked at 1 MiB (0x100000) — the traditional "high memory"
 * start above the BIOS/VGA area that multiboot guarantees is available.
 *
 * Symbol contract with assembly / Rust:
 *   _start            — entry point (32-bit protected mode, in .text)
 *   __bss_start       — start of zero-init data (not including .data.pgtables)
 *   __bss_end         — end of BSS
 *   __stack_top       — initial RSP (grows downward, sits in BSS)
 *   __kernel_end      — first byte past the kernel image
 */

ENTRY(_start)

KERNEL_BASE = 0x100000;     /* 1 MiB — above real-mode / BIOS area */

SECTIONS
{
    . = KERNEL_BASE;

    /* ── Multiboot2 header — must appear within first 32 KiB of the file ── */
    .multiboot2 : ALIGN(8) {
        KEEP(*(.multiboot2))
    }

    /* ── Code ────────────────────────────────────────────────────────────── */
    .text : ALIGN(16) {
        *(.text .text.*)
    }

    /* ── Read-only data ───────────────────────────────────────────────────── */
    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    /* ── Initialised data (including pre-zeroed boot page tables) ─────────── */
    .data : ALIGN(4096) {
        *(.data .data.*)
        *(.data.pgtables)       /* boot PML4/PDPT/PD — pre-zeroed in ELF */
    }

    /* ── Zero-init data + stack ───────────────────────────────────────────── */
    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        /* Kernel stack: 64 KiB, aligned to 16 bytes */
        . = ALIGN(16);
        . += 0x10000;
        __stack_top = .;
        . = ALIGN(8);
        __bss_end = .;
    }

    . = ALIGN(4096);
    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr)
        *(.note .note.*)
        *(.comment)
    }
}
