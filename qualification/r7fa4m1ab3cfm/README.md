# R7FA4M1AB3CFM qualification

The target is the exact `R7FA4M1AB3CFM#AA0` Cortex-M4F used by the UNO R4
Minima path: 256 KiB code flash, 32 KiB SRAM and 8 KiB data flash. The pinned
Arm GNU 13.2.Rel1 and Clang/LLD 18 smokes use the hard-float ABI and exercise vector reset,
compiler-generated Thumb-2/DSP/FPU code, option/startup state, external GPIO,
explicit ICU routing, GPT0/GPT3 interrupt entries and SCI9 UART (`RA4M1\n`).

Implemented functionally are SYSTEM clock/oscillator and module-stop state,
IOPORT, ICU, GPT0/GPT3 counter-overflow slices, SCI9 and watchdog. GPT3 is the
documented 16-bit GPT3 instance at `0x40078300`, with ELC overflow event
`0x075`; PWM, capture, and exact clock timing are not modeled. USB, CAN,
analog/LCD blocks and the UNO R4 Wi-Fi-board bridge are unsupported. VCD
records the selected port, GPT, SCI and ICU/interrupt signals and deterministic
runs are compared byte-for-byte.

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
