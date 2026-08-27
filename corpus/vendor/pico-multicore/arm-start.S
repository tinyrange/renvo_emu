.syntax unified
.thumb
.section .text.start
.global _start
.type _start, %function
.thumb_func
_start:
    ldr r0, =0x20042000
    mov sp, r0
    bl main
    ldr r1, =0xfffffff0
    str r0, [r1]
1:
    bkpt #0
    b 1b
