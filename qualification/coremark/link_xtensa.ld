OUTPUT_ARCH("xtensa")
ENTRY(_start)

MEMORY
{
    IRAM (rx)  : ORIGIN = 0x40370000, LENGTH = 512K
    DRAM (rwx) : ORIGIN = 0x3fc88000, LENGTH = 480K
}

SECTIONS
{
    .text : {
        *(.literal .literal.*)
        KEEP(*(.text.start))
        *(.text .text.*)
        *(.rodata .rodata.*)
    } > IRAM
    .data : { *(.data .data.*) } > DRAM
    .bss (NOLOAD) : ALIGN(4) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(4);
        __bss_end = .;
    } > DRAM
}
