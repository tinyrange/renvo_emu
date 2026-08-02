# Original PIC16F15376 register fixtures

`remu_timer0.c` and `remu_pps.c` are original Renvo Emulator register-level
qualification programs written from the public PIC16F15376 data sheet. They do
not reproduce source from Microchip application notes or SDK examples.

The fixture configures the internal oscillator, RE0, Timer0, and the combined
interrupt path. Qualification stops on the first RE0 rising edge and checks
the Timer0, interrupt, and port signals in VCD. Timing remains a deterministic
functional approximation rather than a cycle-accurate 100 ms measurement. The
PPS fixture maps the documented TMR0 output source (`0x19`) to RA0 and checks
that the peripheral source drives the pin independently of LATA.

The fixture is licensed under Renvo Emulator's `MIT OR Apache-2.0` terms.
