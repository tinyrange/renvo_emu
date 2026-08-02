# Seven-target expansion qualification

This is the implemented support contract for the frozen seven-MCU expansion
plan. “Functional” means deterministic enough for the compiler, firmware,
interrupt, peripheral, and waveform scenarios listed here. It does not mean
cycle accuracy or complete silicon compatibility.

| Exact part | CPU profile | Loaded image | Acceptance slice |
|---|---|---|---|
| ATSAMD21E18A-AU | Cortex-M0+ / Armv6-M | little-endian Arm ELF | vector reset, PORT/EIC input, TC3 IRQ, SERCOM0 TX, PA07 |
| STM32L432KCU6 | Cortex-M4F / Armv7E-M / FPv4-SP-D16 | little-endian Arm ELF | flash alias reset, GPIO, TIM2 IRQ, USART2, QUADSPI indirect/memory-mapped flash, soft/softfp/hard ABI |
| R7FA4M1AB3CFM#AA0 | Cortex-M4F / Armv7E-M / FPv4-SP-D16 | little-endian Arm ELF; Intel HEX inspection | option/startup surface, IOPORT, ICU-routed GPT0 IRQ, SCI9, P111 |
| ATmega328PB-AU | enhanced AVR8 | AVR ELF; Intel HEX inspection | Harvard spaces, PB pin change, Timer0 IRQ, USART0, EEPROM |
| MSP430FR2433IRGE | MSP430 CPUXv2 | MSP430 ELF; Intel HEX inspection | reset vector, FRAM/SRAM, P1 IRQ, Timer_A wake, eUSCI_A0 |
| PIC16F15376-I/PT | enhanced mid-range PIC16 | Intel HEX reconstructed into 14-bit words | XC8 startup, banked/linear RAM, RA input, Timer0 IRQ, EUSART1 |
| EFM8BB52F32G-C-QFN32 | EFM8-flavoured MCS-51 | Intel HEX CODE image | separate CODE/IDATA/XDATA/SFR, crossbar GPIO, Timer0/2, UART0 |

The exact memory maps, reset assumptions, vectors, selected pins, interrupt
routes, evidence revisions, licences, VCD paths, fidelity tiers, and unsupported
blocks are machine-readable in [`evidence/targets.toml`](../evidence/targets.toml).
Each target's qualification README records the unchanged vendor source,
permitted startup/link/harness adaptation, and expected outcome.

## Reproducible acceptance

The stable comprehensive command is:

```sh
scripts/qualify-expansion.sh
```

It verifies the frozen `PLAN.html` hash and every immutable toolchain image,
enforces the 1,500-line production-source ceiling, runs all Rust workspace
tests, reruns the original six-target gate, and qualifies all seven expansion
targets. Independent target jobs run concurrently. Every compiler invocation
uses the corpus runner's read-only, capability-free, `--network=none` Docker
boundary and records compiler command, image ID, inputs, outputs, and hashes.

The command writes
`.remu/qualification/expansion/summary.json` using schema
`remu.expansion-qualification.v1`. The summary contains all seven exact part
identities and their distinct CPU profile, image format, memory map, reset,
interrupt routing, peripheral/VCD evidence, build provenance, vendor evidence,
replay digest, and artifact hashes. The gate fails if repeat result JSON or VCD
differs, an evidence field is missing, an original target regresses, or runtime
is 60 seconds or more on the calibrated host.

## Toolchains and optimization lanes

Arm targets run GCC 13.2.Rel1 and Clang/LLD 18. STM32L432KC additionally runs
soft, softfp, and hard-float GCC output and checks compiler-emitted FPU
instructions. ATmega328PB uses Microchip AVR GCC 7.3.0 plus DFP 3.6.299;
MSP430FR2433 uses TI MSP430 GCC 9.3.1.11 and support files 1.212;
PIC16F15376 uses XC8 4.00 plus DFP 1.31.465; EFM8BB52F32G uses source-built
SDCC 4.5.0. The C corpora run at the documented O0/size/speed lanes and cover
startup, calls, stack use, recursion, switch lowering, native widths,
arithmetic helpers, volatile MMIO, and interrupt prologues.

Register behavior outside the selected slices is either unmapped, retained as
documented startup storage, or explicitly identified as functional/deferred.
No unlisted block is implied to be hardware-accurate.
