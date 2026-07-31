.section .text.start
.global _start
.type _start, @function
_start:
    li s0, 0x20000000
    li sp, 0x20000400
    li s1, 0xa1b2c3d4

    # QingKe XW: c.sb x9,29(x8); c.sh x9,42(x8)
    .2byte 0xbc44
    .2byte 0xb426
    # QingKe XW: c.sbsp x9,13(sp); c.shsp x9,26(sp)
    .2byte 0x86c4
    .2byte 0x85e4

    # QingKe XW: c.lbu x9,29(x8)
    .2byte 0x3c44
    li t0, 0xd4
    bne s1, t0, failed
    # QingKe XW: c.lhu x9,42(x8)
    .2byte 0x3426
    li t0, 0xc3d4
    bne s1, t0, failed
    # QingKe XW: c.lbusp x9,13(sp)
    .2byte 0x8684
    li t0, 0xd4
    bne s1, t0, failed
    # QingKe XW: c.lhusp x9,26(sp)
    .2byte 0x85a4
    li t0, 0xc3d4
    bne s1, t0, failed

    li a0, 0
    j exit
failed:
    li a0, 1
exit:
    li t0, 0xfffffff0
    sw a0, 0(t0)
1:
    ebreak
    j 1b
