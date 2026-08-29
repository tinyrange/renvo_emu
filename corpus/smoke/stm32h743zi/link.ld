OUTPUT_ARCH("arm")
ENTRY(_start)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x08000000, LENGTH = 2M
    RAM   (rwx) : ORIGIN = 0x24000000, LENGTH = 512K
}

SECTIONS
{
    .vectors : { KEEP(*(.vectors)) } > FLASH
    .text : { KEEP(*(.text.start)) *(.text .text.*) *(.rodata .rodata.*) } > FLASH
    .data : { *(.data .data.*) } > RAM AT > FLASH
    .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) } > RAM
    _stack_top = ORIGIN(RAM) + LENGTH(RAM);
}
