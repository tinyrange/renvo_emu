.section .text.start
.global _start
.type _start, @function
_start:
    li sp, REMU_STACK_TOP
    .option push
    .option norelax
    la gp, __global_pointer$
    .option pop
    la t0, __bss_start
    la t1, __bss_end
1:
    bgeu t0, t1, 2f
    sw zero, 0(t0)
    addi t0, t0, 4
    j 1b
2:
    call main
    li t0, 0xfffffff0
    sw a0, 0(t0)
3:
    ebreak
    j 3b
