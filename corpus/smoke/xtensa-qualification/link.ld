OUTPUT_ARCH("xtensa")
ENTRY(_start)

MEMORY
{
    IRAM (rx)  : ORIGIN = 0x40370000, LENGTH = 512K
    DRAM (rwx) : ORIGIN = 0x3fc80000, LENGTH = 512K
    IROM (rx)  : ORIGIN = 0x42000000, LENGTH = 16M
    DROM (r)   : ORIGIN = 0x3c000000, LENGTH = 16M
}

SECTIONS
{
    .text 0x40370000 :
    {
        *(.literal .literal.*)
        KEEP(*(.text.start))
        *(.text .text.*)
        *(.rodata .rodata.*)
    } > IRAM

    .level1_vector_literal 0x4037833c :
    {
        *(.level1_vector.literal)
    } > IRAM

    .level1_vector 0x40378340 :
    {
        KEEP(*(.level1_vector))
    } > IRAM

    .irom 0x42000000 :
    {
        *(.irom.literal)
        *(.irom)
    } > IROM

    .drom 0x3c000000 :
    {
        *(.drom)
    } > DROM

    .data : { *(.data .data.*) } > DRAM
    .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) } > DRAM
}
