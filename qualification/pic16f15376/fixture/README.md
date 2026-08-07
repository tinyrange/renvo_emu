# Original PIC16F15376 register fixtures

`remu_timer0.c` and `remu_nco.c` are original Renvo Emulator register-level
qualification programs written from the public PIC16F15376 data sheet. They do
not reproduce source from Microchip application notes or SDK examples.

The Timer0 fixture configures the internal oscillator, RE0, Timer0, and the
combined interrupt path. Qualification stops on the first RE0 rising edge and
checks the Timer0, interrupt, and port signals in VCD. The NCO fixture programs
NCO1 in fixed-duty mode with a maximum 20-bit increment; qualification stops on
its first output edge and checks the NCO1 signal and interrupt path. Both tests
use deterministic functional timing rather than cycle-accurate measurements.

The fixture is licensed under Renvo Emulator's `MIT OR Apache-2.0` terms.
