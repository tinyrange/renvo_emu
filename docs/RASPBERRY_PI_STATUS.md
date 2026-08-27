# RP2040 and RP2350 status

This document is the review boundary for the Raspberry Pi consolidation. It
distinguishes deterministic functional behavior from register facades and from
silicon timing. The authoritative source inventories are the
[RP2040 datasheet](https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf),
the [RP2350 datasheet](https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf),
and Raspberry Pi's generated Pico SDK register headers.

## Consolidated functional surface

| Block | RP2040 | RP2350 Arm | RP2350 Hazard3 | Functional boundary |
|---|---:|---:|---:|---|
| Shared run control and scheduling | yes | yes | yes | Stable stimulus order, limits, retries, signal stops, and monotonic events |
| UART1 | yes | yes | yes | Audited PL011 registers, enable gating, IDs, and deterministic TX; UART0 retains its existing host RX path |
| ADC | yes | yes | yes | Target channel masks, host samples, FIFO, round-robin, and FIFO interrupt status |
| PWM | 8 slices | 12 slices | 12 slices | Target-specific globals, compare outputs, wrap/raw/forced/masked status |
| DMA | 12 channels, 2 IRQs | 16 channels, 4 IRQs | 16 channels, 4 IRQs | Byte/halfword/word memory transfers, increments, aliases, completion/force/clear, trigger and abort |
| PIO | PIO0/1 | PIO0/1/2 | PIO0/1/2 | Instruction memory, host FIFOs, status/debug/IRQ registers, deterministic `SET` and `JMP` execution |
| SPI | DW SSI0/1 | PrimeCell SSP0/1 | PrimeCell SSP0/1 | Target-correct register layouts, FIFOs, interrupts, loopback, and queued host data |
| I2C | DW I2C0/1 | DW I2C0/1 | DW I2C0/1 | Command/RX FIFOs, host transactions, masks, read-clear state, aliases, and narrow APB access |
| IO bank | 30 connected GPIOs | GPIO0-31 connected | GPIO0-31 connected | Status/control overrides, edge/level state, W1C raw events, enable/force/status, CPU IRQ delivery |
| USB device | yes | existing shared slice | existing shared slice | Controller/DPRAM host flow; RP2040 register masks, reset values, W1C state, aliases, and narrow I/O are audited |
| Watchdog and RTC | yes | no shared claim | no shared claim | RP2040 countdown/reset boundary plus deterministic calendar/alarm and IRQ |
| RP2040 power controls | yes | n/a | n/a | ROSC, PSM, and voltage/reset control slice |
| RP2350 accelerators/control | n/a | yes | yes | HSTX, SHA-256, OTP, TRNG, ACCESSCTRL, TICKS, and POWMAN register/functional slices |

“Yes” means the model has focused contract tests and is mapped into that CPU
mode. It does not mean the block is cycle accurate or feature complete.

## Qualification evidence

- `scripts/docker-smoke.sh` compiles native-address RP2040, RP2350 Cortex-M33,
  and RP2350 Hazard3 firmware. It covers CPU/exception paths plus GPIO, timers,
  UART, SPI, I2C, PIO, IO-bank, and RP2040 USB behavior with JSON, bus-log, and
  VCD assertions where applicable.
- The generated register-coverage gate requires both UART instances, both SPI
  and I2C instances, IO_BANK0, PIO0, SIO, and native timer evidence on each
  target; RP2040 additionally requires its USB controller proof. RP2350 proofs
  are bound independently in both Cortex-M33 and Hazard3 modes.
- `scripts/qualify-rp2040-sdk.sh` fetches byte-exact upstream Pico examples at
  commit `c81c855ffdedc825975a40ba357723a71358ddf0`, verifies their hashes, and
  proves native UART and PWM register behavior.
- `scripts/qualify-rp2040-multicore.sh` runs the pinned upstream multicore FIFO
  example through the native six-word core-1 launch handshake.
- `.github/workflows/raspberry-pi-qualification.yml` runs both pinned suites on
  relevant pull requests, weekly, and on demand, and retains their evidence.

The Pico adapters are intentionally narrow. They provide startup and the small
SDK call surface required by unchanged upstream example sources. Adapter writes
are asserted against the native register addresses so a facade cannot pass by
silently accepting the wrong register.

## Known gaps and next work

### Protocol and signal fidelity

- UART1 does not yet model generated RX/TX interrupts, modem lines, DMA
  handshakes, baud timing, or FIFO timing. Pin-level muxing is not coupled to
  the UART model.
- SPI and I2C are transaction-level host models. Serial clocks, pin arbitration,
  I2C slave/multimaster behavior, SPI slave timing, and DMA pacing remain open.
- PWM outputs are functional compare state rather than divider-accurate GPIO
  waveforms. ADC samples are host values rather than analog pad/temperature
  physics.
- USB PHY signalling, packet timing, controller DMA, exhaustive endpoint edge
  cases, and a complete class catalogue remain outside the present slice.

### DMA, PIO, and IO depth

- DMA currently advances one transfer unit per machine service. Chaining,
  TREQ/pacing timers, ring addressing, sniff, byte swap, quiet terminators,
  security attribution, and bus arbitration are not modeled.
- PIO still needs `WAIT`, `IN`, `OUT`, `PULL`, `PUSH`, side-set, clock dividers,
  DMA requests, and the RP2350 cross-PIO controls.
- RP2350 GPIO32-47 retain a register surface but are not connected to electrical
  nets. Secure/non-secure IO routing, QSPI interrupt behavior, pad electrical
  settings, and dormant wake remain open.

### Security, clocks, and power

- RP2350 ACCESSCTRL and OTP preserve deterministic policy/register state, but
  do not yet enforce TrustZone/security attribution on every bus master.
- POWMAN, TICKS, ROSC, PSM, watchdog, and RTC use deterministic abstract time.
  Oscillator settling, independent clock domains, dormant wake, brownout, and
  complete reset fan-out are not claimed.
- Exact XIP/QMI cache and serial timing requires a separate cycle-fidelity
  design; the current interpreter is deliberately functional.

### SDK acceptance

Issue #28 is only partially satisfied. The pinned public suite has UART TX, PWM,
and multicore coverage, while the repository's own Docker fixtures cover more
controllers. Pinned upstream end-to-end cases are still needed for GPIO IRQ,
UART RX, I2C, SPI, nontrivial PIO, watchdog, USB, and native UF2 boot. RP2350
also needs a pinned Pico SDK 2.x suite that runs the same source in Cortex-M33
and Hazard3 modes where the SDK permits.

## Open-PR lineage consolidated here

The consolidation replaces the overlapping implementation/audit stacks for
run control (#179, #275, #305, #325), scheduling (#183, #271, #306, #326),
UART1 (#45, #132, #270, #319), ADC (#48, #136, #320), PWM (#49, #135, #321),
PIO (#50, #133, #134, #322), SPI (#46, #250, #264, #323), I2C (#47, #245,
#262, #324), IO bank (#54, #246, #263), RP2040 USB (#249, #269), DMA (#55),
watchdog (#58), RTC (#59), power controls (#144), the RP2350 control and
accelerator drafts (#145-#151), and the pinned Pico qualification drafts
(#181, #182).

Superseded branches should be closed only after the consolidation passes its
public checks, so their review history remains available during the transition.
