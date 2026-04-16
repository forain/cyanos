/* x86-64 linker script — Limine boot protocol (UEFI)
 *
 * Limine loads the kernel ELF and jumps to _start already in 64-bit long mode.
 * Linked at 1 MiB (0x100000) — Limine identity-maps physical memory so this
 * address is directly accessible.
 *
 * Symbol contract with assembly / Rust:
 *   _start            — entry point (64-bit, in .text)
 *   __bss_start       — start of zero-init data
 *   __bss_end         — end of BSS
 *   __stack_top       — initial RSP (grows downward, sits in BSS)
 *   __kernel_end      — first byte past the kernel image
 */

ENTRY(_start)

KERNEL_BASE = 0x100000;     /* 1 MiB — well above real-mode / BIOS area */

/*
 * AP Trampoline reservation — 0x7000–0x7FFF (4 KiB)
 * ─────────────────────────────────────────────────
 * arch/x86_64/src/smp.rs copies the 16-bit AP startup trampoline to physical
 * address 0x7000 at runtime (the only sub-1-MiB address the CPU can start in
 * real mode).  Associated data pages live at 0x7100–0x71FF (GDT) and
 * 0x7F00–0x7FFF (per-AP parameters).
 *
 * This range MUST NOT overlap any kernel section.  The ASSERT below enforces
 * that: since KERNEL_BASE = 0x100000 >> 0x8000 this passes by construction,
 * but the check documents the invariant for future changes.
 *
 * Physical layout of the reserved region (written by smp_init):
 *   0x7000–0x703F   16-bit trampoline code (real mode → long mode)
 *   0x7040–0x707F   32-bit continuation code (protected mode)
 *   0x7080–0x70BF   64-bit continuation code (long mode, calls sched_ap_entry)
 *   0x7100–0x712F   Minimal GDT (4 entries × 8 bytes)
 *   0x7120–0x7127   GDTR (6-byte pointer, at 0x7120)
 *   0x7F00–0x7F07   BSP page-table root (CR3 value)
 *   0x7F08–0x7F0F   AP entry function pointer (64-bit VA)
 *   0x7F10–0x7F17   Atomic AP counter (incremented by each AP on entry)
 *   0x7F18+         AP kernel stack tops (one u64 per AP)
 */
AP_TRAMPOLINE_BASE = 0x7000;
AP_TRAMPOLINE_END  = 0x8000;

SECTIONS
{
    . = KERNEL_BASE;

    /* Verify the kernel image does not overlap the AP trampoline region. */
    ASSERT(KERNEL_BASE >= AP_TRAMPOLINE_END,
        "KERNEL_BASE overlaps the AP trampoline region 0x7000-0x7FFF");

    /* ── Code ─────────────────────────────────────────────────────────────── */
    .text : ALIGN(16) {
        *(.text .text.*)
    }

    /* ── Read-only data ───────────────────────────────────────────────────── */
    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    /* ── Limine request structures ────────────────────────────────────────── */
    /* Must be in a writable, non-BSS section so magic numbers survive the    */
    /* BSS clear, and Limine can write back the response pointers.             */
    .limine_requests : ALIGN(8) {
        KEEP(*(.limine_requests))
    }

    /* ── Initialised data ─────────────────────────────────────────────────── */
    .data : ALIGN(4096) {
        *(.data .data.*)
    }

    /* ── Zero-init data + stack ───────────────────────────────────────────── */
    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        /* Kernel stack: 64 KiB, 16-byte aligned */
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
