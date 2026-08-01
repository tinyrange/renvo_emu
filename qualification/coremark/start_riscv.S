.section .text.start
.global _start
.type _start, @function
_start:
    li sp, REMU_STACK_TOP
    .option push
    .option norelax
    la gp, __global_pointer$
    .option pop
    call main
    li t0, 0xfffffff0
    sw a0, 0(t0)
1:
    ebreak
    j 1b
