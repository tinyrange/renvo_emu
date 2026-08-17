/* SPDX-License-Identifier: Apache-2.0 */
.section .text.start,"ax",@progbits
.global _start
.type _start, @function
_start:
    li sp, 0x40880000
    la t0, __bss_start
    la t1, __bss_end
0:
    bgeu t0, t1, 2f
    sw zero, 0(t0)
    addi t0, t0, 4
    j 0b
2:
    call main
1:
    wfi
    j 1b
