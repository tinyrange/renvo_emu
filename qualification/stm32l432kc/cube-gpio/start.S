.syntax unified
.thumb
.section .text.start
.global _start
.type _start, %function
.thumb_func
_start:
    bl main
1:
    bkpt #0
    b 1b
