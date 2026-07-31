.syntax unified
.thumb

.section .vectors, "a"
.global vectors
vectors:
    .word 0x20042000
    .word _start + 1
    .rept 14
    .word 0
    .endr
    .word timer_handler + 1

.section .text.start
.global _start
.type _start, %function
.thumb_func
_start:
    ldr r0, =0xffff0208
    movs r1, #20
    str r1, [r0]
    ldr r0, =0xffff0210
    movs r1, #1
    str r1, [r0]
    wfi
    movs r0, #0
    ldr r1, =0xfffffff0
    str r0, [r1]
1:
    bkpt #0
    b 1b

.type timer_handler, %function
.thumb_func
timer_handler:
    ldr r0, =0xffff0220
    movs r1, #1
    str r1, [r0]
    bx lr
