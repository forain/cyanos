ENTRY(_start)
OUTPUT_FORMAT(elf64-x86-64)

SECTIONS {
    . = 0xffffffff80200000;

    .multiboot2_header ALIGN(8) : AT(ADDR(.multiboot2_header) - 0xffffffff80000000) {
        *(.multiboot2_header*)
    }

    /* .note.Xen section removed due to linker issues */

    .text ALIGN(4K) : AT(ADDR(.text) - 0xffffffff80000000) {
        __text_start = .;
        *(.text*)
    }

    .rodata ALIGN(4K) : AT(ADDR(.rodata) - 0xffffffff80000000) {
        __text_end = .;
        __rodata_start = .;
        *(.rodata*)
        __altcode_start = .;
        KEEP(*(.altcode*))
        __altcode_end = .;
        . = ALIGN(8);
        __altrelocs_start = .;
        KEEP(*(.altrelocs*))
        __altrelocs_end = .;
        __altfeatures_start = .;
        KEEP(*(.altfeatures*))
        __altfeatures_end = .;
    }

    .data ALIGN(4K) : AT(ADDR(.data) - 0xffffffff80000000) {
        __rodata_end = .;
        *(.data*)
        *(.limine_requests*)
        . = ALIGN(4K);
        __bss_start = .;
        *(.bss*)
        . = ALIGN(8);
        __bss_end = .;
    }

    __end = .;
    __kernel_end = .;

    /DISCARD/ : {
        *(.comment*)
        *(.eh_frame*)
        *(.gcc_except_table*)
        *(.note*)
        *(.rel.eh_frame*)
    }
}