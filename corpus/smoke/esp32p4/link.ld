OUTPUT_ARCH("riscv")
ENTRY(_start)

MEMORY
{
    IROM (rx)  : ORIGIN = 0x40000000, LENGTH = 16M
    RAM  (rwx) : ORIGIN = 0x4ff00000, LENGTH = 768K
}

SECTIONS
{
    .text : { KEEP(*(.text.start)) *(.text .text.*) *(.rodata .rodata.*) } > IROM
    .data : { *(.data .data.*) } > RAM AT > IROM
    .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) } > RAM
    _stack_top = ORIGIN(RAM) + LENGTH(RAM);
}
