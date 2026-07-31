.section .text.start,"ax",@progbits
.literal_position
.global _start
.type _start,@function
_start:
    entry a1, 32
    call8 app_main
    movi a2, 0xfffffff0
    movi a3, 0
    s32i a3, a2, 0
1:
    break 0, 0
    j 1b
