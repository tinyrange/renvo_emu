.section .text.start
.global _start
.type _start, @function
_start:
    li sp, 0x20082000
    li s0, 0
    la t0, trap_handler
    csrw mtvec, t0

    # Enable Hazard3 external interrupt 0 through window 0, then MEI globally.
    li t0, 1 << 16
    csrw 0xbe0, t0
    li t0, 1 << 11
    csrs mie, t0
    li t0, 8
    csrs mstatus, t0

    li t0, 0x400b0028
    lw t1, 0(t0)
    addi t1, t1, 20
    li t0, 0x400b0010
    sw t1, 0(t0)
    li t0, 0x400b0040
    li t1, 1
    sw t1, 0(t0)
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
trap_handler:
    li t0, 0x400b003c
    li t1, 1
    sw t1, 0(t0)
    li s0, 1
    mret
