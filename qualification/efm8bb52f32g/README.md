# EFM8BB52F32G qualification

Run `scripts/qualify-efm8bb52f32g.sh` to build the SDCC small-model corpus and
original Renvo Emulator register fixtures in the pinned, network-isolated container,
then execute the generated Intel HEX on the EFM8 machine.

The gate covers all legal base MCS-51 opcodes, SDCC startup/calling convention,
native 16-bit `int` and three-byte generic pointers, recursion, generated
arithmetic helpers, split memory spaces, interrupt prologues, paged SFRs,
GPIO/crossbar, Timer0/2, UART0, SMBus0, PCA0 three-channel PWM/capture/compare,
deterministic replay, coverage and hierarchical VCD signals. The smoke builds
baseline, size-optimized and speed-optimized
lanes and emits `EFM8BB52:OK\nIRQ\n` after reading an externally stimulated
P0.1 input.

The crossbar fixture checks UART0's fixed P0.4/P0.5 priority, `P0SKIP`
handling, and first-free SPI0 assignment. Route decisions and the global
crossbar enable are observable in VCD without implying peripheral waveform
behavior that belongs to the individual peripheral models.

The exact functional register surface is RSTSRC, CLKSEL, WDTCN, Ports 0–3,
port modes and XBR0/XBR1/XBR2 priority allocation with PnSKIP masks, Timer0/2, UART0, SMBus0 control/data/FIFO status,
PCA0 and IE/IP/EIE1/EIP1 routing. PCA0
models the 16-bit timebase, SYSCLK divider selection, edge-aligned 8–11/16-bit
PWM, output polarity, compare flags, capture on sampled rising/falling edges,
and the shared PCA interrupt line. CEX0–CEX2 and the PCA request are exposed as
hierarchical VCD signals. The SMBus slice provides deterministic register
transactions, a host receive queue, service-flag behavior, FIFO status/flush
semantics, the 0x003b interrupt route, and VCD byte/busy/interrupt
observability. It intentionally does not model arbitration or clock-level
waveforms. The machine boots
from CODE address zero with 32 KiB flash, 256-byte IDATA and 2304-byte XRAM;
CODE, IDATA, XDATA, paged SFR and bit-addressable accesses remain distinct.
The flash fixture proves the firmware-visible `FLKEY` authorization sequence,
MOVX programming, and 2 KiB page erase rather than using a host-only mutation
path.
Timing, analog peripherals, SMBus arbitration/clock waveforms and SPI remain
explicitly functional/deferred as stated by the target manifest. PCA timing is
functional on the abstract simulation timeline, not a silicon-clock model.

The register fixtures are original project code written from the public data
sheet and reference manual; no Silicon Labs SDK source is distributed or
required. The expected result is a P1.4 transition through Timer2 while a
separate UART0 and SMBus fixtures prove the SDCC declaration/ABI surface and
SMB0 data path.
