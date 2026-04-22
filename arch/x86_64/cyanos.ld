ENTRY(kstart)
OUTPUT_FORMAT(elf64-x86-64)

KERNEL_OFFSET = 0xFFFFFFFF80000000;

SECTIONS {
    . = 0x100000;

    .text ALIGN(4K) : {
        __text_start = .;
        *(.text*)
        __text_end = .;
    }

    .rodata ALIGN(4K) : {
        __rodata_start = .;
        *(.rodata*)
        __rodata_end = .;
    }

    .data ALIGN(4K) : {
        *(.data*)
        . = ALIGN(4K);
        *(.bss*)
        __bss_start = .;
        __bss_end = .;
    }

    __end = .;

    /DISCARD/ : {
        *(.comment*)
        *(.eh_frame*)
        *(.gcc_except_table*)
        *(.note*)
        *(.rel.eh_frame*)
    }
}