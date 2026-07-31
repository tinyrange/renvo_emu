.section .text.start
.global _start
.type _start, @function
_start:
    li sp, 0x20000800
    la t0, trap_handler
    csrw mtvec, t0
    li t0, 8
    csrs mstatus, t0
    li t0, 128
    csrs mie, t0

    li t0, 0xffff0208
    li t1, 20
    sw t1, 0(t0)
    li t0, 0xffff0210
    li t1, 1
    sw t1, 0(t0)
    wfi

    li a0, 0
    li t0, 0xfffffff0
    sw a0, 0(t0)
1:
    ebreak
    j 1b

.align 2
trap_handler:
    li t0, 0xffff0220
    li t1, 1
    sw t1, 0(t0)
    mret
