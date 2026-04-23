ENTRY(_start)
OUTPUT_FORMAT("elf64-littleaarch64", "elf64-littleaarch64", "elf64-littleaarch64")

PHDRS
{
    limine_requests PT_LOAD;
    text PT_LOAD;
    rodata PT_LOAD;
    data PT_LOAD;
}

SECTIONS
{
    . = 0xffffffff80000000;

    .limine_requests : {
        KEEP(*(.limine_reqs))
    } :limine_requests

    . = ALIGN(4K);

    .text : {
        __text_start = .;
        *(.text .text.*)
        __text_end = .;
    } :text

    . = ALIGN(4K);

    .rodata : {
        __rodata_start = .;
        *(.rodata .rodata.*)
        *(.gcc_except_table*)
        __rodata_end = .;
    } :rodata

    . = ALIGN(4K);

    .data : {
        __data_start = .;
        *(.data .data.*)
        *(.got .got.*)
        *(.data.rel.ro .data.rel.ro.*)
        *(.tdata .tdata.*)
        __data_end = .;
    } :data

    .bss : {
        . = ALIGN(8);
        PROVIDE(__bss_start = .);
        *(.bss .bss.*)
        *(.tbss .tbss.*)
        *(COMMON)
        PROVIDE(__bss_end = .);
    } :data

    __kernel_end = .;

    /DISCARD/ : {
        *(.comment*)
        *(.eh_frame*)
        *(.note*)
    }
}
