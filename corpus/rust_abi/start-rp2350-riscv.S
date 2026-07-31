.section .text.start,"ax"
.global _start
.type _start, @function
_start:
    li sp, 0x20082000
    call rust_main
    li t0, 0xfffffff0
    sw a0, 0(t0)
1:
    ebreak
    j 1b
