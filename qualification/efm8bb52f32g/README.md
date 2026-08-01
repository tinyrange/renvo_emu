# EFM8BB52F32G qualification

Run `scripts/qualify-efm8bb52f32g.sh` to build SDCC small-model corpus lanes and
the selected Silicon Labs SDK examples in the pinned, network-isolated
container, then execute the generated Intel HEX on the EFM8 machine.

The gate covers all legal base MCS-51 opcodes, SDCC startup/calling convention,
native 16-bit `int` and three-byte generic pointers, recursion, generated
arithmetic helpers, split memory spaces, interrupt prologues, paged SFRs,
GPIO/crossbar, Timer0/2, UART0, deterministic replay, coverage and hierarchical
VCD signals. The smoke builds baseline, size-optimized and speed-optimized
lanes and emits `EFM8BB52:OK\nIRQ\n` after reading an externally stimulated
P0.1 input.

The exact functional register surface is RSTSRC, CLKSEL, WDTCN, Ports 0–3,
port modes and crossbar, Timer0/2, UART0 and IE/IP routing. The machine boots
from CODE address zero with 32 KiB flash, 256-byte IDATA and 2304-byte XRAM;
CODE, IDATA, XDATA, paged SFR and bit-addressable accesses remain distinct.
Timing, analog peripherals, PCA, SMBus and SPI remain explicitly
functional/deferred as stated by the target manifest.

The selected Silicon Labs 8051 SDK 4.2.3 sources are pinned through mirror
revision `eff6046fcb1705f00588bcd2d03cc8f0361bfc14` and retain the Silicon
Labs Software License Agreement v11 notice. The Blinky main/Timer2 ISR and
UART ISR logic are unchanged. A tracked SDCC declaration header and startup
adapter replace generated Simplicity Studio files; the expected result is a
P1.4 LED transition while the UART ISR also compiles as an ABI proof.
