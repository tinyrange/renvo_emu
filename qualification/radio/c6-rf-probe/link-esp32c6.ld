/* SPDX-License-Identifier: Apache-2.0 */
OUTPUT_ARCH("riscv")
ENTRY(_start)

MEMORY
{
    IRAM (rwx) : ORIGIN = 0x40800020, LENGTH = 512K - 0x20
}

SECTIONS
{
    .text :
    {
        KEEP(*(.text.start))
        *(.text .text.*)
        *(.rodata .rodata.*)
        *(.data .data.*)
        *(.sdata .sdata.*)
    } > IRAM
    .bss (NOLOAD) :
    {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        __bss_end = .;
    } > IRAM
    /DISCARD/ : { *(.comment) *(.riscv.attributes) }
}
