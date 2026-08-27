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
| DMA | 12 channels, 2 IRQs | 16 channels, 4 IRQs | 16 channels, 4 IRQs | Paced transfers, chaining, rings, byte swap, sniff, quiet completion, error state, target IRQs, and RP2350 channel security context |
| PIO | PIO0/1 | PIO0/1/2 | PIO0/1/2 | All eight instruction families, shift engines, stalls, joined FIFOs, wrap/delay/side-set/dividers, DREQ, pin muxing, and RP2350 GPIOBASE/PUTGET |
| SPI | DW SSI0/1 | PrimeCell SSP0/1 | PrimeCell SSP0/1 | Target-correct register layouts, FIFOs, interrupts, loopback, and queued host data |
| I2C | DW I2C0/1 | DW I2C0/1 | DW I2C0/1 | Command/RX FIFOs, host transactions, masks, read-clear state, aliases, and narrow APB access |
| IO bank and pads | 30 connected GPIOs | 48 connected GPIOs | 48 connected GPIOs | Muxed SIO/PIO drive, four-state nets, weak pulls, input/output-disable, overrides, edge/level state, W1C events, and CPU IRQ delivery |
| USB device | yes | existing shared slice | existing shared slice | Controller/DPRAM host flow plus packet PID/CRC validation, toggles, SOF/reset/suspend/resume, control/bulk stages, NAK, and STALL |
| Watchdog and RTC | yes | no shared claim | no shared claim | RP2040 countdown/reset boundary plus deterministic calendar/alarm and IRQ |
| RP2040 power controls | yes | n/a | n/a | ROSC, PSM, and voltage/reset control slice |
| RP2350 accelerators/control | n/a | yes | yes | HSTX, SHA-256, OTP, TRNG, TICKS, POWMAN, and ACCESSCTRL policy enforced for core and DMA bus contexts |

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
- RP USB packets have deterministic PID, CRC5/CRC16, data-toggle, handshake,
  frame, reset, and suspend behavior. Bit-cell NRZI/bit stuffing, analogue PHY
  signalling, controller timing, isochronous edge cases, and a complete class
  catalogue remain outside the present slice.

### DMA, PIO, and IO depth

- DMA implements documented transfer widths, pacing timers and connected PIO
  DREQs, chaining, ring addressing, sniff modes, byte swap, quiet terminators,
  error state, and per-channel RP2350 security attribution. Arbitration remains
  deterministic at one transfer unit per active channel per machine service;
  non-PIO peripheral DREQ wiring and bus-cycle contention remain open.
- PIO implements `JMP`, `WAIT`, `IN`, `OUT`, `PUSH`/`PULL`, `MOV`, `IRQ`, and
  `SET`, including stalls, auto push/pull, joined FIFO capacities, wrap,
  side-set, delay, 16.8 dividers, DREQs, and pin muxing. RP2350 exposes GPIOBASE
  and processor FIFO PUT/GET windows. PIO v1 state-machine PUT/GET and
  cross-PIO opcode extensions, plus silicon-exact cycle timing, remain open.
- All 48 RP2350 GPIOs have connected four-state nets. Pad pulls are weak and
  yield to strong drives; input-enable, output-disable, IO-bank overrides, and
  SIO/PIO muxing affect the functional signal path. Drive strength, slew rate,
  and Schmitt settings are retained without analogue/timing effects. QSPI IRQs,
  per-pin non-secure masks, and dormant wake remain open.

### Security, clocks, and power

- RP2350 ACCESSCTRL policies and locks are enforced on core and per-channel DMA
  accesses, with explicit secure/non-secure and privileged/unprivileged bus
  contexts. The CPU interpreters do not implement architectural Armv8-M
  Secure/Non-secure transitions, SAU/IDAU/MPU execution, or every debug master;
  tests select those contexts directly and default application execution to
  non-secure privileged access.
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
