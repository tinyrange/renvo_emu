.syntax unified
.thumb

#ifdef RP2040_BOOT2
.section .boot2,"a",%progbits
.space 256, 0
#endif

.section .vectors,"a",%progbits
.word STACK_TOP
.word _start + 1
.rept 14
.word default_handler + 1
.endr

.section .text.start,"ax",%progbits
.global _start
.type _start, %function
.thumb_func
_start:
    ldr r0, =GPIO_CONFIG_ADDRESS
    ldr r1, =GPIO_CONFIG_VALUE
    str r1, [r0]
    ldr r0, =GPIO_SET_ADDRESS
    ldr r1, =GPIO_SET_VALUE
    str r1, [r0]
    ldr r0, =0xfffffff0
    movs r1, #0
    str r1, [r0]
1:
    bkpt #0
    b 1b

.global default_handler
.type default_handler, %function
.thumb_func
default_handler:
    b default_handler
