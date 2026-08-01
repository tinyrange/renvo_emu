OUTPUT_ARCH("xtensa")
ENTRY(_start)

MEMORY
{
    IRAM (rx) : ORIGIN = 0x40370020, LENGTH = 512K - 0x20
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
    /DISCARD/ : { *(.comment) *(.xtensa.info) *(.xt.prop) *(.xt.lit) }
}
