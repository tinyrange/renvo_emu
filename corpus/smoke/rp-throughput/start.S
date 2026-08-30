.syntax unified
.cpu cortex-m0plus
.thumb
.section .text.start
.global _start
.type _start, %function
.thumb_func
_start:
    movs r0, #0
    movs r1, #1
1:
    adds r0, #1
    eors r1, r0
    b 1b
