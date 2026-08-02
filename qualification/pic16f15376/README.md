# PIC16F15376 qualification

The compiler lane uses MPLAB XC8 4.00 with PIC16F1xxxx DFP 1.31.465 and
`-mcpu=16F15376`. `-mstack=reentrant` selects XC8's software data-stack ABI so
the corpus can cover reentrant calls and recursion instead of silently relying
on XC8's default non-reentrant allocation model.

The smoke corpus is compiled at `-O0`, `-Os` and `-O2`. It covers XC8 startup,
native 16-bit `int`, aggregates, volatile data, calls and recursion, switch
lowering, generated arithmetic helpers, GPIO, Timer0 and Timer2 interrupt
entry/return, DAC1 code selection, and EUSART1 output. The register fixtures
additionally exercise the C1 comparator's GPIO input selection and output edge.
Every smoke build must emit
`PIC16F15376:OK\nIRQ\n`.

Implemented functionally: all 49 enhanced mid-range instruction families,
14-bit word program memory, banked/common/linear RAM, the 16-level hardware
stack, reset and interrupt vectors, oscillator-ready state, PORT/LAT/TRIS/
ANSEL A–E, PPS register storage plus functional TMR0/TX1 output-source routing,
PIR/PIE routing, Timer0/1, Timer2 period
matching with its prescaler/postscaler, EUSART1 transmit, MSSP1 SPI master byte
transfers, the normalized DAC1 code and enable/output controls, the C1
comparator GPIO input/polarity/output and edge-flag slice, and watchdog reset.
Timer, serial, SPI, and comparator timing are deterministic approximations.
MSSP1 I²C/slave operation remains unsupported. The comparator is a logic-level
model; it does not simulate analog voltage, propagation delay, or the complete
C2/zero-cross path. Serial receive timing and unlisted peripherals also remain
unsupported and are not represented as hardware-accurate.

Run from the repository root:

```sh
scripts/qualify-pic16f15376.sh
```

That command also assembles the complete 49-instruction fixture and compiles
and runs the original register-level Timer0, Timer2, DAC1, comparator, and PPS
programs described under [`fixture`](fixture/README.md). They are written from
the public PIC16F15376 data sheet and must produce the expected timer, DAC
enable, C1 output, and routed RA0 edges without incorporating Microchip
application-note or SDK source.
