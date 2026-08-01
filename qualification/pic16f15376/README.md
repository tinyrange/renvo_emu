# PIC16F15376 qualification

The compiler lane uses MPLAB XC8 4.00 with PIC16F1xxxx DFP 1.31.465 and
`-mcpu=16F15376`. `-mstack=reentrant` selects XC8's software data-stack ABI so
the corpus can cover reentrant calls and recursion instead of silently relying
on XC8's default non-reentrant allocation model.

The smoke corpus is compiled at `-O0`, `-Os` and `-O2`. It covers XC8 startup,
native 16-bit `int`, aggregates, volatile data, calls and recursion, switch
lowering, generated arithmetic helpers, GPIO, Timer0 interrupt entry/return and
EUSART1 output. Every build must emit `PIC16F15376:OK\nIRQ\n`.

Implemented functionally: all 49 enhanced mid-range instruction families,
14-bit word program memory, banked/common/linear RAM, the 16-level hardware
stack, reset and interrupt vectors, oscillator-ready state, PORT/LAT/TRIS/
ANSEL A–E, PPS register storage, PIR/PIE routing, Timer0/1, EUSART1 transmit and
MSSP1 SPI master byte transfers, and watchdog reset. Timer, serial and SPI
timing are deterministic approximations. MSSP1 I²C/slave operation, analog
modules, serial receive timing and unlisted peripherals remain unsupported and
are not represented as hardware-accurate.

Run from the repository root:

```sh
scripts/qualify-pic16f15376.sh
```

That command also assembles the complete 49-instruction fixture and compiles
and runs the original register-level Timer0 program described under
[`fixture`](fixture/README.md). It is written from the public PIC16F15376 data
sheet and must produce the first RE0 Timer0 edge without incorporating
Microchip application-note or SDK source.
