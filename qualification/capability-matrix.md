# Renvo Emulator capability matrix

This matrix is generated from target manifests and checked qualification artifacts. Tier 3 is a named workflow claim, not arbitrary SDK or production-firmware compatibility.

Capability input SHA-256: `e756d4f9bd5ca2d614a16d0201f3bef28cdd2b9221530984c583b1d3fcd3f9de`

| Target | Highest tier | CPU evidence rows | Native formats | Peripheral scope | Official workflow | Tracker |
| --- | --- | --- | --- | --- | --- | --- |
| [WCH CH32V003](https://github.com/tinyrange/renvo_emu/issues/4) | Selected board or SDK workflow | ch32v003 (raw-bin) | elf, raw-bin | GPIO, TIM2, USART1, PFIC, RCC, VCD | Unmodified WCH EVT GPIO Toggle | https://github.com/tinyrange/renvo_emu/issues/4 |
| [WCH CH32V006](https://github.com/tinyrange/renvo_emu/issues/5) | Selected board or SDK workflow | ch32v006 (raw-bin) | elf, raw-bin | GPIO, TIM2, USART1, PFIC, RCC, VCD | Unmodified WCH EVT GPIO Toggle | https://github.com/tinyrange/renvo_emu/issues/5 |
| [Raspberry Pi RP2040](https://github.com/tinyrange/renvo_emu/issues/6) | Selected board or SDK workflow | rp2040 (uf2) | elf, uf2 | SIO GPIO, UART0, TIMER, PIO0, USB, XIP, VCD | Unmodified Pico SDK blink_simple | https://github.com/tinyrange/renvo_emu/issues/6 |
| [Raspberry Pi RP2350A](https://github.com/tinyrange/renvo_emu/issues/16) | Selected board or SDK workflow | rp2350-arm (uf2); rp2350-riscv (uf2) | elf, uf2 | SIO GPIO, UART0, TIMER, PIO0, USB, XIP, VCD | Unmodified Pico SDK blink_simple on Cortex-M33; Unmodified Pico SDK blink_simple on Hazard3 | https://github.com/tinyrange/renvo_emu/issues/16 |
| [Espressif ESP32-S3](https://github.com/tinyrange/renvo_emu/issues/7) | Selected board or SDK workflow | esp32s3 (esp-bin) | elf, esp-bin | GPIO, UART0, timer groups, USB Serial/JTAG, USB OTG, SPI flash, RNG, VCD | Unmodified ESP-IDF hello_world | https://github.com/tinyrange/renvo_emu/issues/7 |
| [Espressif ESP32-C6](https://github.com/tinyrange/renvo_emu/issues/15) | Selected board or SDK workflow | esp32c6 (esp-bin) | elf, esp-bin | GPIO, UART0, timer groups, USB Serial/JTAG, SPI flash, PMP, VCD | Unmodified ESP-IDF hello_world | https://github.com/tinyrange/renvo_emu/issues/15 |

## Tier definitions

- **Compiler execution** — Compiler-produced code runs through a documented CPU and ABI subset with bounded results and tracing.
- **Firmware functional slice** — Native image handling and a tested subset of chip-specific GPIO, timer, UART, interrupt, and startup behavior.
- **Selected board or SDK workflow** — A named board, SDK path, vendor sample, or official firmware image reaches a tested endpoint; this never implies arbitrary firmware compatibility.

Every tier above is bound to artifact paths and SHA-256 digests in `dashboard.json`; `scripts/check-capability-matrix.sh` rejects stale generated outputs.
