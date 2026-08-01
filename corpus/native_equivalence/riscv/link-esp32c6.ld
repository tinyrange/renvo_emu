OUTPUT_ARCH("riscv")
ENTRY(_start)

MEMORY
{
    IRAM (rwx) : ORIGIN = 0x40800020, LENGTH = 512K - 0x20
}

SECTIONS
{
    .text : { KEEP(*(.text.start)) *(.text .text.*) *(.rodata .rodata.*) } > IRAM
    /DISCARD/ : { *(.comment) *(.riscv.attributes) }
}
