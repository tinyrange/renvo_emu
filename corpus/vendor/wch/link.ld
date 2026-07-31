OUTPUT_ARCH("riscv")
ENTRY(_start)
MEMORY {
    FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 16K
    RAM (rwx) : ORIGIN = 0x20000000, LENGTH = 2K
}
SECTIONS {
    .text : { KEEP(*(.text.start)) *(.text .text.*) *(.rodata .rodata.*) } > FLASH
    .data : { *(.data .data.*) *(.sdata .sdata.*) } > RAM AT > FLASH
    .bss (NOLOAD) : { *(.bss .bss.*) *(.sbss .sbss.*) *(COMMON) } > RAM
}
