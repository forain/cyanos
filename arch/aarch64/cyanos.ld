/* AArch64 linker script for QEMU -machine virt
 *
 * QEMU virt DRAM starts at 0x40000000.  We link the kernel at 0x40080000
 * to leave the first 512 KiB for QEMU's internal firmware/DTB.
 * The kernel entry (_start) must be the very first byte executed.
 *
 * Symbol contract with assembly / Rust:
 *   _start            — entry point (in .text.boot)
 *   __bss_start       — start of zero-initialised data
 *   __bss_end         — end of BSS (8-byte aligned)
 *   __stack_top       — initial SP (grows downward)
 *   __kernel_end      — first byte past the kernel image
 *   __exception_vectors — AArch64 vector table (arch/aarch64/src/exception.rs)
 */

ENTRY(_start)

KERNEL_BASE = 0x40080000;

SECTIONS
{
    . = KERNEL_BASE;

    /* ── Boot stub — must execute before any other .text ─────────────────── */
    .text.boot : {
        KEEP(*(.text.boot))
    }

    /* ── Normal code ─────────────────────────────────────────────────────── */
    .text : ALIGN(4096) {
        *(.text .text.*)
    }

    /* ── Read-only data ───────────────────────────────────────────────────── */
    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    /* ── Initialised read-write data ─────────────────────────────────────── */
    .data : ALIGN(4096) {
        *(.data .data.*)
    }

    /* ── 1 MiB kernel stack (increased for large Task struct operations) ─── */
    .stack : ALIGN(16) {
        __stack_bottom = .;
        . += 0x100000;          /* 1 MiB (increased from 128 KiB for 267KB Task) */
        __stack_top = .;
    }

    /* ── Zero-initialised data ────────────────────────────────────────────── */
    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(8);
        __bss_end = .;
    }

    . = ALIGN(4096);
    __kernel_end = .;

    /* Discard DWARF frame unwinding tables — we don't unwind in the kernel */
    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr)
        *(.note .note.*)
        *(.comment)
        *(.ARM.exidx*)
    }
}
