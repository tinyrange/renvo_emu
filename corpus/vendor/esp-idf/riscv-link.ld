OUTPUT_ARCH("riscv")
ENTRY(_start)
MEMORY {
    IROM (rx) : ORIGIN = 0x42000000, LENGTH = 16M
    DRAM (rwx) : ORIGIN = 0x40800000, LENGTH = 512K
}
SECTIONS {
    .text : { KEEP(*(.text.start)) *(.text .text.*) *(.rodata .rodata.*) } > IROM
    .data : { *(.data .data.*) *(.sdata .sdata.*) } > DRAM AT > IROM
    .bss (NOLOAD) : { *(.bss .bss.*) *(.sbss .sbss.*) *(COMMON) } > DRAM
}
