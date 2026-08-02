# Original EFM8BB52 register fixtures

These files are original Renvo Emulator qualification fixtures written from the public
EFM8BB52 data sheet and reference manual. They do not contain or require source
from the Silicon Labs 8051 SDK.

`remu_blinky.c` and `remu_timer2_irq.c` configure the modeled watchdog,
crossbar, GPIO, Timer2, and interrupt path and stop on a P1.4 transition.
`remu_uart_irq.c`, `remu_uart1_irq.c`, and `remu_timer345_irq.c` separately
prove SDCC's declaration and interrupt ABI for UART0, the paged UART1 vector,
and the extended Timer3/4/5 vectors. `remu_pca.c` compiles
the PCA0 three-channel PWM/compare declaration and
interrupt vector against the same header. `remu_smbus.c` exercises the SMB0
leader-start and data-register path. `remu_adc_irq.c` proves the ADC window and
conversion interrupt declarations.
`remu_dac.c` exercises the DAC0 page-`0x30` register declarations and output
update path.
The small declaration and startup adapters expose only the documented register
surface needed by these fixtures.

All files in this directory are licensed under Renvo Emulator's `MIT OR Apache-2.0`
terms.
