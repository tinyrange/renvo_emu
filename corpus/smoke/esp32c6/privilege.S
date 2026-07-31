.section .text.start
.global _start
.type _start, @function
_start:
    li sp, 0x40880000

    # Basic PMP-visible behavior: machine-mode writes are retained.
    li t0, 0x12345
    csrw pmpaddr0, t0
    csrr t1, pmpaddr0
    bne t0, t1, failed
    li t0, 0x0f
    csrw pmpcfg0, t0
    csrr t1, pmpcfg0
    bne t0, t1, failed

    la t0, trap_handler
    csrw mtvec, t0
    la t0, user_entry
    csrw mepc, t0
    csrr t1, mstatus
    li t0, 0xffffe7ff
    and t1, t1, t0
    csrw mstatus, t1
    mret

user_entry:
    li s0, 0
    ecall
    # This machine CSR access must trap as illegal from user mode.
    csrr t0, mstatus
    li t0, 2
    bne s0, t0, failed
    li a0, 0
    j exit

trap_handler:
    csrr t0, mcause
    li t1, 8
    beq t0, t1, trapped_ecall
    li t1, 2
    beq t0, t1, trapped_csr
    j failed

trapped_ecall:
    bnez s0, failed
    li s0, 1
    j advance_mepc

trapped_csr:
    li t1, 1
    bne s0, t1, failed
    li s0, 2

advance_mepc:
    csrr t0, mepc
    addi t0, t0, 4
    csrw mepc, t0
    mret

failed:
    li a0, 1
exit:
    li t0, 0xfffffff0
    sw a0, 0(t0)
1:
    ebreak
    j 1b
