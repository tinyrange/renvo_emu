.section .text.start
.global _start
.type _start, @function
_start:
    li sp, 0x20002000
    li t0, 0x40000800

    # CH32V006 streamlined TIM3: an internal-clock, five-tick period.
    li t1, 4
    sh t1, 0x0c(t0)
    li t1, 2
    sh t1, 0x10(t0)
    li t1, 3
    sh t1, 0x18(t0)
    li t1, 1 << 11
    sh t1, 0x04(t0)
    li t1, 1
    sh t1, 0x00(t0)

    li t2, 8
1:
    lh s1, 0x08(t0)
    addi t2, t2, -1
    bnez t2, 1b

    li a0, 0
    lh s1, 0x0c(t0)
    li t1, 4
    bne s1, t1, 3f
    lh s1, 0x10(t0)
    li t1, 2
    bne s1, t1, 3f
    j 4f
3:
    li a0, 1
4:
    li t0, 0xfffffff0
    sw a0, 0(t0)
2:
    ebreak
    j 2b
