ENTRY(_start)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x10000000, LENGTH = 16M
    RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 256K
}

SECTIONS
{
    .text : { KEEP(*(.text.start)) *(.text .text.*) *(.rodata .rodata.*) } > FLASH
    .data : { *(.data .data.*) } > RAM AT > FLASH
    .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) } > RAM
    /DISCARD/ : { *(.ARM.exidx*) *(.ARM.extab*) *(.comment) }
}
