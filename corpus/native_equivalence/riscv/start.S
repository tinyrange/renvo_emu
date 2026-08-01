#ifdef RP2350_IMAGE
.section .image_def,"a",@progbits
.space 32, 0
.word _start
.word 0x20082000
.space 88, 0
#endif

.section .text.start,"ax",@progbits
.global _start
.type _start, @function
_start:
    li sp, STACK_TOP
    li t0, GPIO_CONFIG_ADDRESS
    li t1, GPIO_CONFIG_VALUE
    sw t1, 0(t0)
    li t0, GPIO_SET_ADDRESS
    li t1, GPIO_SET_VALUE
    sw t1, 0(t0)
    li t0, 0xfffffff0
    sw zero, 0(t0)
1:
    ebreak
    j 1b
