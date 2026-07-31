.section .vectors, "ax"
.global _start
.type _start, @function
_start:
    j reset
    .rept 37
    .word default_handler
    .endr
    .word tim2_handler

.section .text.start
reset:
    li sp, 0x20000800
    li s0, 0

    # Select the QingKe table interrupt ABI rooted at the vector table.
    li t0, 3
    csrw mtvec, t0
    csrw 0x804, t0

    # PFIC IENR[1], TIM2_IRQn = 38 (word 1, bit 6).
    li t0, 0xe000e104
    li t1, 1 << 6
    sw t1, 0(t0)

    # TIM2: (PSC + 1) * (ATRLR + 1) = 10 abstract ticks.
    li t0, 0x40000000
    li t1, 1
    sh t1, 0x28(t0)
    li t1, 4
    sh t1, 0x2c(t0)
    sh zero, 0x10(t0)
    li t1, 1
    sh t1, 0x0c(t0)
    sh t1, 0x00(t0)

    li t0, 8
    csrs mstatus, t0
    wfi

    li a0, 1
    beqz s0, finish
    li a0, 0
finish:
    li t0, 0xfffffff0
    sw a0, 0(t0)
1:
    ebreak
    j 1b

.align 2
tim2_handler:
    li t0, 0x40000000
    sh zero, 0(t0)
    li t1, -2
    sh t1, 0x10(t0)
    li s0, 1
    mret

default_handler:
    li a0, 2
    li t0, 0xfffffff0
    sw a0, 0(t0)
2:
    ebreak
    j 2b
