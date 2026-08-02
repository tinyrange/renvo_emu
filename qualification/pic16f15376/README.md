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
ANSEL A–E, PPS register storage, PIR/PIE routing, Timer0/1, EUSART1 transmit,
the MSSP1 I²C host byte path (7-bit addresses, START/RESTART/STOP, queued
reads, SSP1IF, typed native register IDs, BF/WCOL/SSPOV, ACKSTAT, and ACKEN),
and watchdog reset. Timer and serial timing are deterministic approximations.
The I²C model reports deterministic byte-level transactions through the
peripheral handle, including configurable address ACK/NACK responses; it does
not model SCL/SDA edge timing, arbitration, 10-bit addressing, or slave-mode
operation. Analog modules, serial receive timing and unlisted peripherals
remain unsupported and are not represented as hardware-accurate.

The MSSP1 audit follows the native register summary and I²C host transmission
and reception descriptions in Microchip DS40001866E, with the device-specific
register window at `0x018c..0x0192`. Command bits are modeled as
single-operation strobes, writes while the receive buffer is full set the
documented overflow/collision diagnostics, and functional transfers complete
immediately at the abstract simulation timestamp.

Run from the repository root:

```sh
scripts/qualify-pic16f15376.sh
```

That command also assembles the complete 49-instruction fixture and compiles
and runs the original register-level Timer0 program described under
[`fixture`](fixture/README.md). It is written from the public PIC16F15376 data
sheet and must produce the first RE0 Timer0 edge without incorporating
Microchip application-note or SDK source.
