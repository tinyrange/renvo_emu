# Original PIC16F15376 timer, serial, analog, PPS, and NCO fixtures

`remu_timer0.c`, `remu_timer2.c`, `remu_dac.c`, `remu_comparator.c`, and
`remu_pps.c`, and `remu_nco.c` are original Renvo Emulator register-level qualification programs
written from the public PIC16F15376 data sheet. They do not reproduce source
from Microchip application notes or SDK examples.

The Timer0 fixture configures the internal oscillator, RE0, Timer0, and the
combined interrupt path, including MSSP1 as a 7-bit I²C host. The Timer2
fixture configures the programmable period,
1:1 prescaler/postscaler, and the same interrupt path. Qualification stops on
the first RE0 rising edge and checks the timer, interrupt, and port signals in
VCD. The DAC fixture selects a 5-bit code and enables DAC1; qualification stops
on the DAC active signal and checks the normalized code in VCD. The comparator
fixture selects C1IN0- on RA0 and C1IN0+ on RA2; the host supplies their digital
levels, and qualification stops on the resulting comparator output edge. Timer
and comparator timing remain deterministic functional approximations, and DAC
output is intentionally represented as a digital code rather than a voltage.
The PPS fixture maps the documented TMR0 output source (`0x19`) to RA0 and
checks that the peripheral source drives the pin independently of LATA.
The NCO fixture programs NCO1 in fixed-duty mode with a maximum 20-bit
increment; qualification stops on its first output edge and checks the NCO1
signal and interrupt path.

The fixture is licensed under Renvo Emulator's `MIT OR Apache-2.0` terms.
