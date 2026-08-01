.section .text.start
.global _start
.type _start, @function
_start:
    la sp, __stack_top
    call remu_case
    li t0, 0xfffffff0
    sw a0, 0(t0)
1:
    ebreak
    j 1b
