OUTPUT_ARCH("arm")
ENTRY(_start)

MEMORY
{
    FLASH (rx) : ORIGIN = 0x10000000, LENGTH = 16M
    RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 520K
}

SECTIONS
{
    .vectors : { KEEP(*(.vectors)) } > FLASH
    .text : { KEEP(*(.text.start)) *(.text .text.*) *(.rodata .rodata.*) } > FLASH
    /DISCARD/ : { *(.comment) *(.ARM.attributes) }
}
