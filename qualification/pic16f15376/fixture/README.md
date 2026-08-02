# Original PIC16F15376 timer and DAC fixtures

`remu_timer0.c`, `remu_timer2.c`, and `remu_dac.c` are original Renvo Emulator
register-level qualification programs written from the public PIC16F15376 data
sheet. They do not reproduce source from Microchip application notes or SDK
examples.

The Timer0 fixture configures the internal oscillator, RE0, Timer0, and the
combined interrupt path. The Timer2 fixture configures the programmable period,
1:1 prescaler/postscaler, and the same interrupt path. Qualification stops on
the first RE0 rising edge and checks the timer, interrupt, and port signals in
VCD. The DAC fixture selects a 5-bit code and enables DAC1; qualification stops
on the DAC active signal and checks the normalized code in VCD. Timer timing
remains a deterministic functional approximation, and DAC output is
intentionally represented as a digital code rather than a voltage.

The fixture is licensed under Renvo Emulator's `MIT OR Apache-2.0` terms.
