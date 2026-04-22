ENTRY(_start)
OUTPUT_FORMAT("elf64-littleaarch64", "elf64-littleaarch64", "elf64-littleaarch64")

SECTIONS {
    . = 0xffffffff80080000;

    .text : AT(ADDR(.text) - 0xffffffff80000000) {
        __text_start = .;
        *(.text*)
	. = ALIGN(4096);
        __text_end = .;
    }

    .rodata : AT(ADDR(.rodata) - 0xffffffff80000000) {
        __rodata_start = .;
        *(.rodata*)
	. = ALIGN(4096);
        __rodata_end = .;
    }

    .data : AT(ADDR(.data) - 0xffffffff80000000) {
        *(.data*)
        *(.limine_requests*)
	. = ALIGN(4096);
        __bss_start = .;
        *(.bss*)
        . = ALIGN(8);
        __bss_end = .;
	. = ALIGN(4096);
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