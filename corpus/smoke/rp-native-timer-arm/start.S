.syntax unified
.thumb

#ifndef TIMER_BASE
#error "TIMER_BASE must select the target timer"
#endif
#ifndef TIMER_INTR_OFFSET
#error "TIMER_INTR_OFFSET must select the target interrupt status register"
#endif
#ifndef TIMER_INTE_OFFSET
#error "TIMER_INTE_OFFSET must select the target interrupt enable register"
#endif

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
    ldr r0, =TIMER_BASE + 0x28
    ldr r1, [r0]
    adds r1, #20
    ldr r0, =TIMER_BASE + 0x10
    str r1, [r0]
    ldr r0, =TIMER_BASE + TIMER_INTE_OFFSET
    movs r1, #1
    str r1, [r0]
    wfi

    cmp r4, #1
    bne failure
    movs r0, #0
    b finish
failure:
    movs r0, #1
finish:
    ldr r1, =0xfffffff0
    str r0, [r1]
1:
    bkpt #0
    b 1b

.type timer_handler, %function
.thumb_func
timer_handler:
    ldr r0, =TIMER_BASE + TIMER_INTR_OFFSET
    movs r1, #1
    str r1, [r0]
    movs r4, #1
    bx lr
