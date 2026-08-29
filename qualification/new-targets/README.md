# STM32F411RE, nRF52840, and ESP32-P4 qualification

This directory defines the first evidence-backed software-emulation slice for
the three targets added after the original thirteen-MCU portfolio. The stable
gate is:

```sh
scripts/qualify-new-targets.sh
```

The gate builds freestanding firmware with immutable Docker toolchains, runs
each ELF twice, and requires byte-identical result JSON and VCD. Firmware must
use native-address GPIO, UART, and timer registers before writing zero to the
shared compiler-test exit word. A timestamp-zero host stimulus drives input pin
3 high; the firmware fails if it cannot observe that value through the native
GPIO input register.

| Target | CPU contract | Native slice | Expected UART |
|---|---|---|---|
| STM32F411RE | Cortex-M4F / Armv7E-M | flash alias, RCC/FLASH startup, GPIOA, USART2, TIM2 | `STM32F411\n` |
| nRF52840 | Cortex-M4F / Armv7E-M | P0/P1 GPIO, UART0 tasks/events, TIMER0 compare | `NRF52840\n` |
| ESP32-P4 | RV32IMAC subset of the HP core | L2/TCM/XIP map, GPIO, UART0, timer group 0 | `ESP32P4\n` |

The scope is functional and deterministic rather than cycle accurate. The
machine-readable contract and source revisions are in
[`manifest.json`](manifest.json), and the precise modeled register semantics are
in [`registers.md`](registers.md). Native raw-image versus direct-ELF
equivalence is additionally enforced by `scripts/qualify-native-images.sh`.
No radio device, RF frame, physical board, or hardware-backed test is used by
this qualification.
