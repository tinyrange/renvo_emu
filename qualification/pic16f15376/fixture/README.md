# Original PIC16F15376 Timer0 fixture

`renvo_timer0.c` is an original Renvo register-level qualification program
written from the public PIC16F15376 data sheet. It does not reproduce source
from Microchip application notes or SDK examples.

The fixture configures the internal oscillator, RE0, Timer0, and the combined
interrupt path. Qualification stops on the first RE0 rising edge and checks
the Timer0, interrupt, and port signals in VCD. Timing remains a deterministic
functional approximation rather than a cycle-accurate 100 ms measurement.

The fixture is licensed under Renvo's `MIT OR Apache-2.0` terms.
