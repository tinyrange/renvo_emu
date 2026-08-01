# TI SLAC700 evidence

The qualification gate stages three source files unchanged from TI's official
`SLAC700` package, version `01.00.00.0E` (1 July 2020):

- `msp430fr243x_P1_01.c` — polled P1.3 input and P1.0 output;
- `msp430fr243x_ta0_02.c` — Timer0_A CCR0 interrupt and P1.0 toggle;
- `msp430fr243x_euscia0_uart_03.c` — eUSCI_A0 transmit/receive loopback.

The archive is downloaded from TI and must match SHA-256
`9d8b339b98949afd26a31609d16baed06257eeb06126043ac0a3eda09397e12d`.
All three examples are compiled by the pinned TI MSP430 GCC container with the
official FR2433 header, linker script, startup object and support library.

The GPIO example receives P1.3 stimulus from the host. The timer example runs
until its first ISR-driven falling edge. The UART example's documented physical
TX-to-RX jumper is represented by the small link-time wrapper in this directory:
it enables the eUSCI `UCLISTEN` internal loopback bit before entering TI's
unchanged `main`. Multiple incrementing transmitted bytes prove that receive
interrupts wake the original loop and that it did not stop after its first TX.
