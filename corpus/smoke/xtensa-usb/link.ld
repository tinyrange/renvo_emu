OUTPUT_ARCH("xtensa")
ENTRY(_start)

MEMORY
{
    IRAM (rx)  : ORIGIN = 0x40370000, LENGTH = 512K
    DRAM (rwx) : ORIGIN = 0x3fc80000, LENGTH = 512K
}

SECTIONS
{
    .text :
    {
        *(.literal .literal.*)
        KEEP(*(.text.start))
        *(.text .text.*)
        *(.rodata .rodata.*)
    } > IRAM

    .data : { *(.data .data.*) } > DRAM
    .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) } > DRAM
}
