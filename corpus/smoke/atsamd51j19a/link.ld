OUTPUT_ARCH("arm")
ENTRY(_start)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 512K
    RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 192K
}

SECTIONS
{
    .vectors : { KEEP(*(.vectors)) } > FLASH
    .text : { KEEP(*(.text.start)) *(.text .text.*) *(.rodata .rodata.*) } > FLASH
    .data : { *(.data .data.*) } > RAM AT > FLASH
    .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) } > RAM
    _stack_top = ORIGIN(RAM) + LENGTH(RAM);
}
