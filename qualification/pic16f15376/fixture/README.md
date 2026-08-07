# Original PIC16F15376 register fixtures

`remu_timer0.c` and `remu_comparator.c` are original Renvo Emulator register-level
qualification programs written from the public PIC16F15376 data sheet. They do
not reproduce source from Microchip application notes or SDK examples.

The Timer0 fixture configures the internal oscillator, RE0, Timer0, and the
combined interrupt path. Qualification stops on the first RE0 rising edge and
checks the Timer0, interrupt, and port signals in VCD. The comparator fixture
selects C1IN0- on RA0 and C1IN0+ on RA2; the host supplies their digital levels,
and qualification stops on the resulting comparator output edge. Timing remains
a deterministic functional approximation rather than a cycle-accurate
measurement.

The fixture is licensed under Renvo Emulator's `MIT OR Apache-2.0` terms.
