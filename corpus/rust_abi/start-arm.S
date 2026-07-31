.syntax unified
.thumb
.section .text.start,"ax"
.global _start
.type _start, %function
.thumb_func
_start:
    ldr r0, =0x20040000
    mov sp, r0
    bl rust_main
    ldr r1, =0xfffffff0
    str r0, [r1]
1:
    bkpt #0
    b 1b
