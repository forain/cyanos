/* AArch64 linker script for QEMU -machine virt
 *
 * QEMU virt DRAM starts at 0x40000000.  We link the kernel at 0xffffffff40080000
 * (higher-half mapping) which Limine maps to physical 0x40080000.
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

KERNEL_BASE = 0xffffffff40080000;
KERNEL_PHYS = 0x40080000;

SECTIONS
{
    . = KERNEL_BASE;

    /* ── Boot stub — must execute before any other .text ─────────────────── */
    .text.boot : AT(KERNEL_PHYS) {
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

    /* ── Limine request structures ────────────────────────────────────────── */
    .limine_requests : ALIGN(8) {
        KEEP(*(.limine_requests))
    }

    /* ── Initialised read-write data ─────────────────────────────────────── */
    .data : ALIGN(4096) {
        *(.data .data.*)
    }

    /* ── Zero-initialised data + stack ───────────────────────────────────── */
    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        /* Kernel stack: 1 MiB, 16-byte aligned (increased for large Task struct operations) */
        . = ALIGN(16);
        . += 0x100000;
        __stack_top = .;
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