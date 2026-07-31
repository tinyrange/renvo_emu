OUTPUT_ARCH("riscv")
ENTRY(_start)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x42000000, LENGTH = 4M
    RAM   (rwx) : ORIGIN = 0x40800000, LENGTH = 512K
}
__stack_top = ORIGIN(RAM) + LENGTH(RAM);

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
    } > RAM AT > FLASH
    .bss (NOLOAD) : {
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
    } > RAM
}
