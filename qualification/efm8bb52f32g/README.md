# EFM8BB52F32G qualification

Run `scripts/qualify-efm8bb52f32g.sh` to build the SDCC small-model corpus and
original Renvo Emulator register fixtures in the pinned, network-isolated container,
then execute the generated Intel HEX on the EFM8 machine.

The gate covers all legal base MCS-51 opcodes, SDCC startup/calling convention,
native 16-bit `int` and three-byte generic pointers, recursion, generated
arithmetic helpers, split memory spaces, interrupt prologues, paged SFRs,
GPIO/crossbar, Timer0/2, UART0, deterministic replay, coverage and hierarchical
VCD signals. The smoke builds baseline, size-optimized and speed-optimized
lanes and emits `EFM8BB52:OK\nIRQ\n` after reading an externally stimulated
P0.1 input.

The exact functional register surface is RSTSRC, CLKSEL, PCON0/PCON1/REG0CN,
WDTCN, Ports 0–3, port modes and crossbar, Timer0/2, UART0 and IE/IP routing.
CLKSEL reports source/divider and nominal SYSCLK transitions through VCD;
PCON0/PCON1 enters deterministic IDLE/STOP/SNOOZE/SHUTDOWN modes. The machine boots
from CODE address zero with 32 KiB flash, 256-byte IDATA and 2304-byte XRAM;
CODE, IDATA, XDATA, paged SFR and bit-addressable accesses remain distinct.
Oscillator settle timing, pin-retention wake, analog peripherals, PCA, SMBus
and SPI remain explicitly functional/deferred as stated by the target manifest.

The register fixtures are original project code written from the public data
sheet and reference manual; no Silicon Labs SDK source is distributed or
required. The expected result is a P1.4 transition through Timer2 while a
separate UART0 interrupt fixture proves the SDCC declaration and ABI surface.
