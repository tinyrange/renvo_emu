.syntax unified
.thumb

.section .vectors, "a"
.global vectors
vectors:
    .word 0x20042000
    .word _start + 1
    # Exceptions 2 through 14.
    .rept 13
    .word 0
    .endr
    .word systick_handler + 1
    # Exceptions 16 through 48. External IRQ 33 is exception 49.
    .rept 33
    .word 0
    .endr
    .word nvic_handler + 1

.section .text.start
.global _start
.type _start, %function
.thumb_func
_start:
    movs r4, #0

    # SysTick: reload 3, clear current, enable counter and interrupt.
    ldr r0, =0xe000e014
    movs r1, #3
    str r1, [r0]
    ldr r0, =0xe000e018
    movs r1, #0
    str r1, [r0]
    ldr r0, =0xe000e010
    movs r1, #7
    str r1, [r0]
    wfi

    # Exercise NVIC bank 1 using software-pended external interrupt 33.
    ldr r0, =0xe000e104
    movs r1, #2
    str r1, [r0]
    ldr r0, =0xe000ef00
    movs r1, #33
    str r1, [r0]
    nop

    cmp r4, #3
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

.type systick_handler, %function
.thumb_func
systick_handler:
    movs r4, #1
    # Disable SysTick so it cannot re-pend during the second proof.
    ldr r0, =0xe000e010
    movs r1, #0
    str r1, [r0]
    bx lr

.type nvic_handler, %function
.thumb_func
nvic_handler:
    adds r4, #2
    bx lr
