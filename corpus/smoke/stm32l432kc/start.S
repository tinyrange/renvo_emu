.syntax unified
.thumb
#ifdef __clang__
#define VECTOR(symbol) .word symbol
#else
#define VECTOR(symbol) .word symbol + 1
#endif
.section .vectors,"a",%progbits
.word _stack_top
VECTOR(_start)
.rept 14
VECTOR(default_handler)
.endr
.rept 28
VECTOR(default_handler)
.endr
VECTOR(tim2_handler)

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
