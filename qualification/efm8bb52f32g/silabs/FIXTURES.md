# Original EFM8BB52 register fixtures

These files are original Renvo qualification fixtures written from the public
EFM8BB52 data sheet and reference manual. They do not contain or require source
from the Silicon Labs 8051 SDK.

`renvo_blinky.c` and `renvo_timer2_irq.c` configure the modeled watchdog,
crossbar, GPIO, Timer2, and interrupt path and stop on a P1.4 transition.
`renvo_uart_irq.c` separately proves SDCC's declaration and interrupt ABI for
UART0. The small declaration and startup adapters expose only the documented
register surface needed by these fixtures.

All files in this directory are licensed under Renvo's `MIT OR Apache-2.0`
terms.
