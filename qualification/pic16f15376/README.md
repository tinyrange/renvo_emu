# PIC16F15376 qualification

The compiler lane uses MPLAB XC8 4.00 with PIC16F1xxxx DFP 1.31.465 and
`-mcpu=16F15376`. `-mstack=reentrant` selects XC8's software data-stack ABI so
the corpus can cover reentrant calls and recursion instead of silently relying
on XC8's default non-reentrant allocation model.

The smoke corpus is compiled at `-O0`, `-Os` and `-O2`. It covers XC8 startup,
native 16-bit `int`, aggregates, volatile data, calls and recursion, switch
lowering, generated arithmetic helpers, GPIO, Timer0 interrupt entry/return and
EUSART1 output. The register fixtures additionally exercise the C1 comparator's
GPIO input selection and output edge. Every smoke build must emit
`PIC16F15376:OK\nIRQ\n`.

Implemented functionally: all 49 enhanced mid-range instruction families,
14-bit word program memory, banked/common/linear RAM, the 16-level hardware
stack, reset and interrupt vectors, oscillator-ready state, PORT/LAT/TRIS/
ANSEL A–E, PPS register storage, PIR/PIE routing, Timer0/1, the C1 comparator
GPIO input/polarity/output and edge-flag slice, EUSART1 transmit and watchdog
reset. Timer, comparator and serial timing are deterministic approximations.
The comparator is a logic-level model; it does not simulate analog voltage,
propagation delay or the complete C2/zero-cross path. Other analog modules,
serial receive timing and unlisted peripherals remain unsupported and are not
represented as hardware-accurate.

Run from the repository root:

```sh
scripts/qualify-pic16f15376.sh
```

That command also assembles the complete 49-instruction fixture and compiles
and runs the original register-level Timer0 and comparator programs described
under [`fixture`](fixture/README.md). They are written from the public
PIC16F15376 data sheet and must produce the first RE0 Timer0 edge and C1 output
edge without incorporating Microchip application-note or SDK source.
