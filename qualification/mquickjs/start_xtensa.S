.section .text.start
.global _start
.type _start, @function
_start:
    call8 main
    movi a3, 0xfffffff0
    s32i a10, a3, 0
1:
    break 0, 0
    j 1b
