# R7FA4M1AB3CFM qualification

The target is the exact `R7FA4M1AB3CFM#AA0` Cortex-M4F used by the UNO R4
Minima path: 256 KiB code flash, 32 KiB SRAM and 8 KiB data flash. The pinned
Arm GNU 13.2.Rel1 and Clang/LLD 18 smokes use the hard-float ABI and exercise vector reset,
compiler-generated Thumb-2/DSP/FPU code, option/startup state, external GPIO,
explicit ICU routing, GPT0 interrupt entry, ADC140 scan completion and SCI9
UART (`RA4M1\n`).

Implemented functionally are SYSTEM/MSTP, IOPORT, ICU, ELC, KINT, GPT0-7,
AGT0-1, SCI9, RSPI0-1, IIC0-1, RTC, ADC140, DAC12, CRC, DOC, CAC, POEG and the
watchdog startup surface. KINT models KR00-KR07 on P100-P107, selectable
rising/falling levels, KRM enables and KRF latching at `0x40080000`; its ICU
event is `0x045`. GPT0-2 use their native 32-bit counters and GPT3-7 their
native 16-bit counters. ELC link registers accept the documented nine-bit
event IDs and software events `0x053`/`0x054`.

ADC140 accepts deterministic host-driven 14-bit samples on AN000..AN028,
performs group-A single scans, supports 8/10/12/14-bit formatting, and emits
scan-end event `0x029`. DAC12 exposes its guest output code in VCD. RTC alarm,
timer, KINT, ADC and SCI events route only through configured ICU IELSR slots.
Continuous/group-B ADC operation, window comparison, physical analog timing,
USB, CAN, LCD behavior, exact clock timing and the UNO R4 Wi-Fi-board bridge
remain unsupported. VCD records the selected port, KINT, GPT, ADC, DAC, ELC,
SCI and ICU/interrupt signals; deterministic runs are compared byte-for-byte.

The vendor lanes build pinned Renesas FSP GPT/SCI peripheral sources unchanged
with documented BSP/startup adapters. They also compile and run pinned Arduino
Blink and MultiSerial sketches; Blink stops on the P111 edge and MultiSerial
emits `H`. The exact upstream revisions are in `evidence/targets.toml`.

The unchanged FSP example sources come from `ra-fsp-examples` revision
`01a411dfc2e9808f489070c780a554a5bead6714` under BSD-3-Clause. The FSP API
surface is pinned at `a409855a274402f69360a725656944e17929d1d9`.
ArduinoCore-renesas is pinned at `424e86eff92d37f72123c2b641dd8bbf06a38b47`
under MIT and the Blink/MultiSerial sketches at
`ad14bc44cb95555e5df7c16e6605559cad860d29` under CC0-1.0. Local BSP,
startup, link, and host adapters are documented in the staged source; expected
results are a P111 edge, the FSP transcript, and `H` from HardwareSerial.

Run:

```sh
scripts/qualify-r7fa4m1ab3cfm.sh
```
