.syntax unified
.thumb
.section .vectors,"a",%progbits
.word _stack_top
.word _start + 1
.rept 166
.word default_handler + 1
.endr

.section .text.start
.global _start
.type _start, %function
.thumb_func
_start:
    bl main
    ldr r1, =0xfffffff0
    str r0, [r1]
1:
    bkpt #0
    b 1b

.global default_handler
.type default_handler, %function
.thumb_func
default_handler:
    b default_handler
