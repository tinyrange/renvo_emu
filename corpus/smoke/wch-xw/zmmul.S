.section .text.start
.global _start
.type _start, @function
_start:
    li t0, 0x13579bdf
    li t1, 17
    # QingKe V2C Zmmul: mul t2,t0,t1
    .word 0x026283b3
    li t0, 0x48d159cf
    bne t2, t0, failed
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
