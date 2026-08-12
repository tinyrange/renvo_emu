.section .text.start
.global _start
.type _start, @function
_start:
    movi a2, __bss_start
    movi a3, __bss_end
    movi a4, 0
1:
    beq a2, a3, 2f
    s32i a4, a2, 0
    addi a2, a2, 4
    j 1b
2:
    call8 main
    movi a3, 0xfffffff0
    s32i a10, a3, 0
3:
    break 0, 0
    j 3b
