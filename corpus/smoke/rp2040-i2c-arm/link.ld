OUTPUT_ARCH("arm")
ENTRY(_start)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x10000000, LENGTH = 16M
    RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 264K
}

SECTIONS
{
    .text :
    {
        KEEP(*(.vectors))
        KEEP(*(.text.start))
        *(.text .text.*)
        *(.rodata .rodata.*)
    } > FLASH
    .data : { *(.data .data.*) } > RAM AT > FLASH
    .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) } > RAM
}
