# Original EFM8BB52 register fixtures

These files are original Renvo Emulator qualification fixtures written from the public
EFM8BB52 data sheet and reference manual. They do not contain or require source
from the Silicon Labs 8051 SDK.

`remu_blinky.c` and `remu_timer2_irq.c` configure the modeled watchdog,
crossbar, GPIO, Timer2, and interrupt path and stop on a P1.4 transition.
`remu_uart_irq.c` and `remu_uart1_irq.c` separately prove SDCC's declaration
and interrupt ABI for UART0 and the paged UART1 vector. The small declaration
and startup adapters expose only the documented register surface needed by
these fixtures.

All files in this directory are licensed under Renvo Emulator's `MIT OR Apache-2.0`
terms.
