OUTPUT_ARCH("riscv")
ENTRY(_start)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 16K
    RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 2K
}

SECTIONS
{
    .text : {
        KEEP(*(.text.start))
        *(.text .text.*)
        *(.rodata .rodata.*)
    } > FLASH
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
        PROVIDE(__global_pointer$ = . + 0x800);
    } > RAM
    .bss (NOLOAD) : ALIGN(4) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        . = ALIGN(4);
        __bss_end = .;
    } > RAM
    ASSERT(. <= ORIGIN(RAM) + LENGTH(RAM) - 512,
           "CH32V003 CoreMark image leaves less than 512 bytes for stack")
}
