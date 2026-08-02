# Original PIC16F15376 Timer fixtures

`remu_timer0.c` and `remu_timer2.c` are original Renvo Emulator register-level
qualification programs written from the public PIC16F15376 data sheet. They do
not reproduce source from Microchip application notes or SDK examples.

The Timer0 fixture configures the internal oscillator, RE0, Timer0, and the
combined interrupt path. The Timer2 fixture configures the programmable period,
1:1 prescaler/postscaler, and the same interrupt path. Qualification stops on
the first RE0 rising edge and checks the timer, interrupt, and port signals in
VCD. Timing remains a deterministic functional approximation rather than a
cycle-accurate measurement.

The fixture is licensed under Renvo Emulator's `MIT OR Apache-2.0` terms.
