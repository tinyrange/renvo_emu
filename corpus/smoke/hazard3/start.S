.section .text.start
.global _start
.type _start, @function
_start:
    li sp, 0x20082000

    # Zba
    li t0, 5
    li t1, 7
    sh2add t2, t0, t1
    li t3, 27
    bne t2, t3, fail

    # Zbb
    li t0, -8
    li t1, 3
    max t2, t0, t1
    bne t2, t1, fail
    rori t2, t1, 1
    li t3, 0x80000001
    bne t2, t3, fail

    # Zbs
    li t0, 5
    li t1, 7
    bset t2, t0, t1
    li t3, 0x85
    bne t2, t3, fail

    # Zbkb
    li t0, 0x1234
    li t1, 0xabcd
    pack t2, t0, t1
    li t3, 0xabcd1234
    bne t2, t3, fail

    # Execute the function compiled under the complete documented -march.
    li a0, 0x12345678
    li a1, 5
    call hazard3_compiler_cases
    li t0, 0x468acfdb
    bne a0, t0, fail

    # The callee executes explicit Zcb and Zcmp encodings.
    call hazard3_compressed_cases
    li t0, 0xa5
    bne a0, t0, fail

pass:
    li a0, 0
    j exit
fail:
    li a0, 1
exit:
    li t0, 0xfffffff0
    sw a0, 0(t0)
1:
    ebreak
    j 1b
