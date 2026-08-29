OUTPUT_ARCH("xtensa")
ENTRY(_start)

MEMORY
{
    IRAM (rx)  : ORIGIN = 0x40370020, LENGTH = 512K - 0x20
    DROM (r)   : ORIGIN = 0x3c010020, LENGTH = 64K - 0x20
    IROM (rx)  : ORIGIN = 0x42020020, LENGTH = 64K - 0x20
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
    .drom : { *(.drom .drom.*) } > DROM
    .irom :
    {
        *(.irom.literal .irom.literal.*)
        *(.irom .irom.*)
    } > IROM
    /DISCARD/ : { *(.comment) *(.xtensa.info) *(.xt.prop) *(.xt.lit) }
}
