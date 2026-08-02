# Renvo Emulator support contract

This document describes implemented behavior, not the long-term intent in
`PLAN.html`. “Functional” means deterministic and useful for the named corpus;
it does not mean cycle accuracy or complete silicon compatibility.

## Portfolio

| Target | Runnable CPU mode | Direct-load memory | Chip-facing proof |
|---|---|---|---|
| CH32V003 | QingKe-flavoured RV32EC/Zicsr subset | 16 KiB flash, 2 KiB SRAM | RCC + native GPIO, USART1, TIM2, PFIC and table-mode interrupt proofs |
| CH32V006 | QingKe-flavoured RV32EC/Zicsr subset | 64 KiB flash, 8 KiB SRAM | Native WCH RCC/GPIO/USART1/TIM2/PFIC slice with independently sized map |
| RP2040 | Cortex-M0+ Armv6-M Thumb subset | 16 MiB XIP window, 264 KiB SRAM | SIO/IO_BANK0 GPIO; UART0/1; TIMER; SPI0/1; I²C0/1; ADC; PWM; DMA; PIO0/1; USB; watchdog/RTC; and ROSC/PSM/VREG controls |
| RP2350 | Cortex-M33 Thumb subset or Hazard3 RV32IMAC/B subset | 16 MiB XIP window, 520 KiB SRAM | SIO/IO_BANK0 GPIO; UART0/1; TIMER0/1; SPI0/1; I²C0/1; ADC; PWM; DMA; PIO0/1/2; USB; and deterministic accelerator/control slices in both CPU modes |
| ESP32-S3 | Xtensa LX7 windowed compiler subset | DRAM, IRAM, 16 MiB IROM and DROM windows | Windowed ABI/exception/atomic/FPU qualification; GPIO/UART proof plus functional I2C, SPI, I2S, and bidirectional RMT transactions; complete M5StickS3 non-radio board workflow |
| ESP32-C6 | RV32IMAC/Zicsr HP and LP cores | ROM, HP/LP SRAM, 16 MiB IROM window | Complete non-radio MMIO inventory, functional serial/timing/motor/audio/DMA/SDIO/analog/security slices, PMU/cache control, machine/user PLIC and CLINT, staged watchdog resets, user traps, and PMP enforcement |

ATmega328PB SPI0 and SPI1 are modeled through their native
`SPCRn`/`SPSRn`/`SPDRn` registers (`0x4c..0x4e` and `0xac..0xae`). Master writes
are captured independently, host-injected MISO bytes are returned,
`SPIF`/write-collision status is deterministic, and `SPIE` routes completion to
CPU lines 16 and 38. Double-speed timing and package pin/SS arbitration remain
outside this slice. Addresses and vector routing follow Microchip's
[ATmega328PB data sheet](https://ww1.microchip.com/downloads/aemDocuments/documents/MCU08/ProductDocuments/DataSheets/40001906C.pdf)
and [interrupt table](https://onlinedocs.microchip.com/oxy/GUID-0EC909F9-8FB7-46B2-BF4B-05290662B5C3-en-US-12.1.1/GUID-F3266720-5DBF-4EA7-876C-81574D15CD24.html).

EFM8BB52F32G also models the native SPI0 SFR transaction slice
(`SPI0CFG`/`SPI0CKR`/`SPI0CN0`/`SPI0DAT`): master writes are captured,
host-injected MISO bytes are returned, `SPIF`/`TXNF` status is exposed, and
the ESPI0 interrupt participates in the low/high priority interrupt inputs.
FIFO operation, crossbar pin assignment, and exact serial clock timing remain
outside this functional model.

The STM32L432KC functional slice also maps I2C1 at `0x40005400` and I2C3 at
`0x40005C00`. Each controller supports deterministic master START/STOP,
programmable transfer counts, TXDR/RXDR transactions, injected target bytes,
BUSY/TC/STOPF status, and event-interrupt routing (I2C1 event line 31 and I2C3
event line 72). This is a transaction-level model: alternate-function pin
muxing, clock stretching, arbitration, DMA, and electrical open-drain
resolution remain outside the current slice.

All targets also expose a stable compiler-test block:

- GPIO at `0xffff0000`
- UART at `0xffff0100`
- functional timer at `0xffff0200`
- exit word at `0xfffffff0`

This block is explicitly a compiler facade, separate from chip register
compatibility. It lets architecture tests share stopping and observation
conventions without pretending that vendor peripherals are interchangeable.

### RP2350 TRNG subset

Both RP2350 CPU modes map the official TRNG block at `0x400f0000`. Firmware
can configure the source and sample counter, enable generation, poll status,
consume all six result words, clear status, and receive external interrupt 39.
Generation is immediate and reproducible for deterministic CI and replay. It
is not a security entropy source and does not claim the analogue TRNG's
statistical properties or variable completion latency.

### RP2350 SHA-256 subset

Both RP2350 CPU modes map the SHA-256 accelerator at `0x400f8000`. The
functional model accepts byte, halfword, and word input writes, implements CSR
start/status and byte swapping, and exposes all eight read-only digest words.
Complete 512-bit blocks are processed immediately; DMA handshakes, timing, and
security-domain policy are outside this functional slice.

### RP2350 HSTX subset

Both RP2350 CPU modes map HSTX control at `0x400c0000` and its FIFO at
`0x50600000`. The model covers the control CSR, eight lane selectors and
inverters, command-expander configuration, FIFO status/overflow, atomic
aliases, deterministic word consumption, and VCD-visible positive/negative
lane values. Clock-divider latency, DMA pacing, TMDS encoding, and physical
GPIO muxing are not modeled.

### RP2350 OTP subset

Both RP2350 CPU modes map OTP at `0x40120000`. The model exposes page locks,
SBPI status/control, the user data gate, interrupt windows, and ECC/raw/guarded
read aliases over a deterministic read-only image. Lock bits are monotonic
until reset. Fuse programming, security-domain filtering, and ECC fault
injection are deliberately not simulated.

### RP2350 ACCESSCTRL subset

Both RP2350 CPU modes map ACCESSCTRL at `0x40060000`. It models documented
peripheral reset permissions, GPIO non-secure masks, `FORCE_CORE_NS`, atomic
aliases, configuration reset, and write-once locks. Shared-bus guards enforce
the selected secure/non-secure and privileged/unprivileged context before both
normal and fast-path accesses. Core 0, core 1, and each DMA channel select their
own master context; DMA channel `SECCFG` therefore affects the transfer itself,
not only register state. Application execution defaults to non-secure
privileged access, while tests and embedding APIs may select other contexts.

This is deterministic bus-policy enforcement, not a complete Armv8-M security
architecture. The Arm interpreter does not execute Secure/Non-secure state
transitions or SAU/IDAU/MPU attribution, debug-master coverage is incomplete,
and the GPIO non-secure masks are retained but not yet enforced per pin.

### RP2350 TICKS subset

The block at `0x40108000` models the six documented generators for PROC0,
PROC1, TIMER0, TIMER1, WATCHDOG, and RISC-V. Each has enable/running state, a
nine-bit cycles-per-tick divider, a simulation-time-derived countdown, and RP
atomic aliases. Clock-source frequency changes and interrupt generation are
outside this slice.

### RP2350 POWMAN subset

POWMAN at `0x40100000` models reset/configuration state, power-sequencer
requests, GPIO wake descriptors, watchdog reset selection, scratch and boot
words, interrupt state, and its 64-bit always-on timer/alarm. Scratch and boot
words survive modeled watchdog reset. Analog regulator behavior and exact
low-power clock timing are not claimed.

### RP2040 power and oscillator subset

The RP2040 map includes deterministic functional models for the official
`PSM_BASE` (`0x40010000`), `ROSC_BASE` (`0x40060000`), and
`VREG_AND_CHIP_RESET_BASE` (`0x40064000`) blocks. PSM force-on/force-off,
watchdog selection, and `DONE` masks expose power-state transitions. ROSC
implements protected enable/range and drive-strength writes, dormant/wake,
divider and phase controls, stable/enabled status, deterministic `RANDOMBIT`,
and the short `COUNT` delay against abstract simulation ticks. VREG/BOD fields,
immediate functional regulation status, and the write-one-clear restart flag
are available at their documented offsets.

This is a software-visible model: it does not claim analogue voltage curves,
process/voltage/temperature frequency drift, exact oscillator startup delay,
or automatic reset and clock gating of every dependent block.

The STM32L432KC model additionally maps the native bxCAN controller at
`0x40006400`. Its functional slice supports initialization state, loopback
bit-timing selection, one transmit mailbox, receive FIFO 0, standard/extended
identifier fields, payload words, completion/error flags, and maskable status
interrupts. It does not claim bit-level arbitration or a physical CAN bus.

The STM32L432KC model additionally maps the native DAC1 controller at
`0x40007400`. Its functional slice supports both 12-bit channels, right- and
left-aligned plus 8-bit data writes, software-triggered transfers, and
trace-visible digital output/enable signals. It does not claim analog voltage,
calibration, sample-and-hold settling, or DMA behavior.

The STM32L432KC model additionally maps the native TIM1 advanced-control
timer at `0x40012c00`. Its functional slice supports the time-base, update
interrupt, four PWM compare channels, three complementary outputs, main-output
enable, and update generation, with each channel exposed to VCD. It does not
claim cycle-accurate dead-time, break/capture behavior, DMA, or alternate-
function pin routing.

## Official MicroPython milestone

Official, unmodified MicroPython v1.28.0 firmware reaches its native USB raw
REPL on NanoC6, AtomS3 Lite, Pico, and both Pico 2 CPU modes. The current
acceptance matrix proves deterministic portable runtime behavior, soft reset,
threads, persistent storage, external GPIO stimulus, and periodic/one-shot
timer callbacks across all five execution profiles. RP2040, both RP2350 timer
blocks, ESP32-C6 timer groups, and both ESP32-S3 timer groups have distinct
functional register layouts and interrupt delivery.

The host transport counts raw-REPL execution terminators and returned prompts,
so a bounded run ends only after every queued chunk has executed. Instruction
limits remain hard failure bounds rather than the normal completion mechanism.
See `scripts/qualify-micropython.sh` and
`qualification/acceptance-report.html`.

This milestone does not yet cover the complete upstream MicroPython suite,
complete PWM/serial-bus behavior or watchdog resets. ESP radio qualification uses an
isolated deterministic RF medium; it deliberately does not connect firmware to
a host network or claim physical-air fidelity.

RP2040 and RP2350 PWM blocks are mapped at their native addresses and use named
register IDs. The functional model covers per-slice CSR/DIV/CTR/CC/TOP
registers, channel-enable aliases, compare outputs, self-clearing phase
advance/retard commands, wrap interrupt status, write-one-to-clear raw status,
and both RP2350 interrupt banks. It uses the
RP2040 eight-slice global layout (`EN` at `0xa0`) and RP2350's twelve-slice
layout (`EN` at `0xf0`, with the second IRQ bank through `0x10c`). Abstract time
advances enabled counters deterministically; exact divider and phase-correct
edge timing, GPIO pin muxing, DMA pacing, and interrupt controller delivery
remain outside this functional slice. The register layout is checked against
the official [RP2040 datasheet](https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf)
and [RP2350 datasheet](https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf).

RP2350 SPI0 and SPI1 are mapped at their documented `0x4008_0000` and
`0x4008_8000` bases. The functional PrimeCell SSP slice covers the eight-word
transmit/receive FIFOs, DSS/loopback/enable controls, status flags, masked and
raw FIFO interrupts, interrupt clears, peripheral IDs, and deterministic host
input/output. It also follows the RP2350 APB access contract for byte/halfword
lane reads, replicated narrow writes, and the XOR/SET/CLEAR aliases on ordinary
writable registers; CPSDVSR writes are normalised to the documented even
`2..254` range. It intentionally does not model serial-clock waveforms, DMA
handshakes, receive-timeout scheduling, or exact slave-mode timing.

## RP2040 SPI closure

RP2040 SPI0 and SPI1 are mapped at `0x4003_c000` and `0x4004_0000` using the
native Synopsys DW_apb_ssi contract rather than the RP2350 PrimeCell layout.
The typed register model covers `CTRLR0/1`, `SSIENR`, `SER`, baud and FIFO
threshold/level registers, all 36 `DR` windows, status, raw/masked interrupts,
read-clear interrupt registers, DMA controls, identification/version values,
and RP atomic aliases. Functional transfers require SSI and a slave-select
bit, complete in one abstract operation, and either consume queued host input
or loop back the transmitted frame. Serial-clock waveforms, DMA handshakes,
pin muxing, and exact serial timing remain outside this slice. The model uses
the documented sixteen-entry FIFOs and latches transmit-overflow,
receive-overflow, and receive-underflow status with the native clear-on-read
registers. Register
offsets and reset values are audited against the [RP2040 datasheet](https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf)
and Raspberry Pi's generated [`ssi.h`](https://raw.githubusercontent.com/raspberrypi/pico-sdk/master/src/rp2040/hardware_regs/include/hardware/regs/ssi.h).
The `rp2040-spi` Docker fixture compiles and runs the same checks against both
native controller instances. Masked controller status is delivered to the
documented NVIC IRQs 18 and 19.

The RP2350 I²C slice exposes the documented DW_apb_i2c register/FIFO command
path, including reset values, disabled-only configuration writes, sixteen-entry
TX/RX FIFO status, read-clear interrupt registers, component identification,
RP2350 atomic XOR/set/clear write aliases, RP2350 narrow-write replication,
deterministic host-provided read bytes, and VCD byte/strobe signals. The
`IC_INTR_MASK` bits follow the silicon contract: zero masks a source and one
unmasks it. It is a functional host model; electrical SDA/SCL resolution,
arbitration, slave mode, DMA handshakes, and exact bus timing remain outside
the current support contract.

The RP2040 I²C0/I²C1 slice uses the same native DW_apb_i2c register contract at
`0x4004_4000` and `0x4004_8000`. It has the official reset values and access
masks, disabled-only configuration, sixteen-entry command/receive FIFOs,
read-clear interrupt sources, APB byte/halfword lanes, RP atomic aliases, and
deterministic host-provided read bytes. `scripts/docker-smoke.sh` runs a
native-address Cortex-M0+ firmware proof on both controllers. As with RP2350,
pin-level SDA/SCL behavior, arbitration, slave mode, DMA handshakes, interrupt
controller delivery, and exact bus timing are intentionally not claimed.

The RP2040 watchdog model covers `CTRL`, `LOAD`, `REASON`, all eight scratch
registers, `TICK`, atomic aliases, divider/countdown behavior, forced and timed
reset reasons, and scratch persistence. The Arm scheduler stops at the modeled
reset boundary with an explicit watchdog-reset reason. Debug pause inputs,
`RESETS.WDSEL` fan-out, and a complete CPU reboot sequence remain outside the
functional slice.

The shared RP DMA model uses the target's native layout: RP2040 exposes twelve
channels and two interrupt banks, while RP2350 exposes sixteen channels and
four interrupt banks in both Cortex-M33 and Hazard3 modes. It covers aligned
byte/halfword/word copies, address increments, completion status, interrupt
enable/force/clear, atomic aliases, multi-channel trigger, abort, chaining,
read/write ring addressing, byte swap, quiet completion, sniff accumulation,
and documented pacing timers. PIO TX/RX DREQs are connected to FIFO state.
RP2350 channel security configuration selects the ACCESSCTRL bus context used
by each transfer. Transfers advance one unit per active channel per machine
service; exact arbitration, bus contention, and non-PIO peripheral DREQ wiring
remain outside this functional slice.

RP2040 IO_BANK0 covers masked per-pin STATUS/CTRL fields, input/output/enable
override reporting, packed raw edge/level events, W1C event acknowledgement,
and both processor enable/force/status windows. External pin transitions update
this state deterministically, and PROC0 masked/forced status is delivered
through the documented NVIC IRQ 13. SIO and PIO mux selection drives the shared
four-state GPIO nets, with INOVER/OUTOVER/OEOVER applied on the functional path.
PADS_BANK0 pull-up/pull-down, input-enable, and output-disable settings also
affect those nets. QSPI interrupts and dormant wake are not modeled; drive
strength, slew, and Schmitt settings are retained without analogue effects.

RP2350's IO_BANK0 model covers the SDK-facing per-pin STATUS and CTRL
registers, input/output/enable overrides, packed raw edge/level events, and
PROC0/PROC1 enable, force, and status registers. Both Cortex-M33 and Hazard3
modes route PROC0 pending state to IO IRQ line 21. It honors the RP2350 atomic
register aliases and replicated byte/halfword writes. GPIO0-47 have connected
four-state nets; the native high SIO registers, PIO GPIOBASE windows, SIO/PIO
mux selection, overrides, and pad pulls/input/output gates affect their signal
path. Per-pin secure/non-secure mask enforcement, QSPI IRQ routing, dormant
wake, and analogue drive/slew/Schmitt behavior are not yet modeled.

The ESP32-C6 and ESP32-S3 USB Serial/JTAG models expose a deterministic host
connection control surface. They start connected for existing console fixtures;
tests can call `set_usb_host_connected(false)` to exercise a disconnected,
non-blocking console. A connected host asserts `INT_RAW` SOF bit 1 every 1,000
abstract ticks, and `INT_CLR`/raw writes clear the latched status. This is a
functional USB-frame model, not a claim of USB clock or PHY accuracy. The C6
model additionally exposes the documented CONF0/TEST raw-PHY controls, host
line injection and transition timestamps. Its low-speed packet oracle checks
NRZI, stuffing, PID, CRC16, EOP and firmware instruction cadence without
claiming analog-pad or bus-arbitration fidelity. Main-watchdog system resets
perform the functional flash/application handoff again while preserving the
LP-AON stores, allowing the C6 USB recovery window to be qualified end to end.

The ESP32-S3 DWC2 device link has a descriptor-driven deterministic host for
CDC ACM, HID, CDC Ethernet, MIDI, Audio, WebUSB/vendor, MTP, ADB and MSC. The
qualification firmware performs class-specific control and data transactions,
including a 192-byte isochronous audio packet and MSC BOT INQUIRY/CSW; this is
functional controller qualification rather than electrical USB timing.

The ESP32-C6 machine maps every public non-radio register block in the ESP-IDF
v6.0.2 inventory at its vendor address. GPIO/IO MUX; UART0/1 and LP UART; UHCI;
SPI2; I2C0 and LP I2C; I2S; RMT; LEDC; MCPWM; PCNT; PARLIO; TWAI0/1; SDIO
slave; timer groups and system timer; GDMA; SAR ADC/temperature; ETM; staged
watchdogs; PMU, LP-AON and LP timer; EXTMEM cache maintenance; machine/user PLIC
and CLINT; and the AES, SHA, RSA, ECC, HMAC, digital-signature and eFuse blocks
have deterministic functional slices. The RV32IMAC LP core executes from
retained LP SRAM and participates in modeled sleep, wake and reset transitions.
Remaining analog, trace, debug and monitor/APM blocks are strict target-specific
register facades where no behavioral contract is claimed. ESP32-C6 Wi-Fi,
Bluetooth LE, and IEEE 802.15.4 now use a shared deterministic RF medium with
native MMIO, DMA/shared-memory, and interrupt paths. The IEEE 802.15.4
qualification independently executes freestanding firmware and genuine
ESP-IDF public APIs, requiring exact TX, FCS-bearing RX with RSSI/LQI metadata,
energy detection, interrupt clearing, and byte-identical replay. This is
functional LLE coverage, not an analog PHY or host-network bridge. See
`docs/ESP32C6_PERIPHERALS.md` for the fidelity matrix and omissions.

ESP32-C6 and ESP32-S3 genuine BLE-controller qualification covers hopped
connections, LL version/feature/data-length procedures, ACL/L2CAP ATT traffic,
remote PHY request/response and instant-based bidirectional 1M-to-2M updates,
the public PHY-complete callback, remote termination, scan restart, and exact
deterministic replay. Impossible or overlapping PHY instants stop through the
radio legality validator rather than being silently accepted.

## RP2040 RTC slice

The RP2040 model includes a functional calendar and alarm block at the native
`0x4005_c000` address. It supports the documented named register map, setup and
load sequencing, the one-second divider, leap-day and month/year rollover,
ordered `RTC_0`/`RTC_1` reads, raw/enable/force/status alarm state, and delivery
of the masked alarm as the machine's RTC interrupt line. The implementation
uses abstract simulation time, so it is deterministic and useful for firmware
tests but is not a clock-frequency or low-power-domain model.

Coverage includes calendar carry across leap days, read latching, alarm clear
and force behavior, and machine-level interrupt polling. Dormant-mode wake,
clock-source switching, power/reset-domain details, and silicon-specific
electrical behavior remain outside this functional slice. The register
semantics are based on the [RP2040 datasheet](https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf).

The RP2040 USB controller models deterministic VBUS detection, device connect,
bus reset, setup reception, buffer completion, raw/masked interrupt mapping,
and the RP2040 write-clear semantics for `SIE_STATUS`, `BUFF_STATUS`,
`EP_ABORT_DONE`, and `EP_STATUS_STALL_NAK`. The register slice applies the
official control/status masks, reset values, self-clearing SIE commands,
read-only status preservation, atomic aliases, and replicated narrow I/O
writes. The packet link validates PID complements, token/SOF CRC5 and data
CRC16, records control and bulk transactions, maintains endpoint data toggles,
and exposes ACK/NAK/STALL, frame, reset, suspend/resume, and line state. The
[official RP2040 datasheet](https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf)
defines these register contracts. Bit-cell NRZI/bit stuffing, analogue PHY
signalling, exact packet timing, isochronous edge cases, and a complete
class/protocol catalogue remain outside this functional slice.

## ATmega328PB external-interrupt slice

The ATmega328PB model covers all three pin-change interrupt groups through
`PCICR`, `PCIFR`, `PCMSK0..2`, and the package Port B/C/D inputs. It also models
INT1 edge/level sensing through `EICRA`, `EIMSK`, and `EIFR`, with distinct AVR
interrupt lines and VCD request signals. Flags remain latched until the
documented write-one-to-clear operation.

This is a deterministic digital-input model; asynchronous electrical timing,
sleep wake-up latency, and peripheral alternate-function pin muxing are not
claimed. Register behavior follows the
[ATmega328PB datasheet](https://ww1.microchip.com/downloads/en/devicedoc/40001906a.pdf).

## Implemented CPU surface

The RISC-V interpreter covers RV32I/E integer execution, common compressed
instructions, M and A where selected by the profile, Zicsr, WFI/MRET, basic
machine interrupt entry, QingKe PFIC table-mode entry, all eight QingKe XW
compressed byte/halfword memory operations, the V2C multiply-only Zmmul
subset, and the compiler-facing Hazard3 bit-manipulation subset. Exact PFIC
nesting/HPE remains unsupported. ESP32-C6 supports machine/user transitions,
user ECALL, illegal privileged-CSR traps, delegated interrupt entry, MRET/URET,
and TOR/NA4/NAPOT PMP permission enforcement with architectural access faults.
`qualification/riscv-cpu.json` contains
Docker and negative-profile proofs for XW, Zmmul, ESP privilege/trap behavior,
and the complete RP2350 compiler `-march` baseline.

The Arm interpreter covers the 16-bit Thumb compiler baseline, BL, Thumb-2 and
DSP forms observed in Cortex-M33 output, FPv5 single-precision compiler output,
stack operations, four-state run stops, WFI, functional exception
stacking/vectoring, and EXC_RETURN. The private-peripheral model includes a
deterministic SysTick and all eight NVIC enable/pending banks (240 external
lines). It does not cover the full Armv8-M instruction set, TrustZone, MPU,
NVIC priority/preemption, or complete system-control register surface.

The Xtensa interpreter covers 16-bit density and 24-bit compiler instruction
forms, core integer ALU and comparisons, multiply/divide, SAR-based shifts,
branches, jump-table dispatch, and zero-overhead loops. It implements logical
register windows for the Espressif toolchain's default windowed ABI, level-one
interrupt entry and `RFE`, `S32C1I`/`SCOMPARE1` atomics, and the
single-precision FPU operations emitted by the qualification workload.
`qualification/xtensa-cpu.json` records pinned-GCC proofs at `-O0`, `-O2`, and
`-Os`, including byte-identical repeat runs and IRAM/DRAM/IROM/DROM execution.
Precise window-overflow traps, complete interrupt priority/nesting, and the
full optional Xtensa ISA remain outside the functional baseline.

## MSP430FR2433 eUSCI_A1 slice

The MSP430FR2433 model exposes the native eUSCI_A1 register window beginning at
`0x0520`: `UCA1CTLW0` (`0x0520`), `UCA1STATW` (`0x052a`), `UCA1RXBUF`
(`0x052c`), `UCA1TXBUF` (`0x052e`), `UCA1IE` (`0x053a`), `UCA1IFG`
(`0x053c`), and `UCA1IV` (`0x053e`). Clearing reset and writing `UCA1TXBUF`
captures a deterministic transmit transcript. Setting `UCLISTEN` in `UCA1STATW`
routes the byte back to `UCA1RXBUF` after the functional serial delay, and
`UCA1IE`/`UCA1IFG` deliver the eUSCI_A1 interrupt vector at `0xffe2`.

The model emits `board.msp430fr2433.uart1.tx_byte` and
`board.msp430fr2433.uart1.tx_strobe` for VCD and signal assertions. This is a
functional UART/loopback slice rather than a complete serial peripheral:
eUSCI_A1 IrDA, pin-multiplexing, SPI mode, and exact bit timing remain deferred.
Register addresses and reset semantics are based on the [MSP430FR2433 data
sheet](https://www.ti.com/lit/ds/symlink/msp430fr2433.pdf) and the [MSP430FR2xx/
FR4xx user's guide](https://www.ti.com/lit/ug/slau445/slau445.pdf).

## MSP430FR2433 eUSCI_B0 SPI slice

The native eUSCI_B0 window starts at `0x0540`, with control, receive/transmit,
interrupt-enable, interrupt-flag, and interrupt-vector registers through
`0x056e`. In synchronous master mode, writing `UCB0TXBUF` completes a
deterministic full-duplex transfer, records MOSI, and places an injected MISO
byte (or an echo when none is injected) in `UCB0RXBUF`. Enabled flags deliver
the eUSCI_B0 vector at `0xffe0`.

Signals expose SPI transmit, receive, and transfer-strobe values under
`board.msp430fr2433.spi0`. I2C protocol state, pin multiplexing, chip-select
wiring, and exact serial timing remain deferred. Addresses and reset semantics
follow the TI MSP430FR2433 data sheet and family user's guide linked above.

## MSP430FR2433 ADC10 slice

The ADC10 model covers the native `0x0700` control window, memory result at
`0x0712`, and interrupt registers at `0x071a`–`0x071e`. A software-triggered,
single-channel conversion samples the host-provided 10-bit channel value after
a deterministic four-tick delay, updates busy/completion state, and raises the
ADC vector at `0xffde` when enabled. Signals expose the sample and end-of-
conversion event under `board.msp430fr2433.adc0`.

Sequences, comparator windows, internal-reference electrical behavior,
capacitive touch, pin multiplexing, and analog timing remain outside this
functional model.

ESP32-S3 DRAM and IRAM power on with the deterministic nonzero byte pattern
`0xa5`. Direct ELF loading copies only each segment's file-backed bytes, so
the synthesized `.bss` tail remains poisoned until firmware clears it. This
keeps direct ELF tests from accidentally depending on loader-provided zeroing.

Verified-image handoff starts with IROM instruction fetch disabled; the ROM
`rom_config_instruction_cache_mode(0x4000, 8, 32)` entry point enables it and
publishes the corresponding `CACHE_STATE` value. A fetch before that call
stops with a diagnostic instead of appearing to work through a coherent-cache
shortcut.

Verified ESP32-S3 application images enter through a modeled `CALLX8`
windowed-ABI handoff. The first application instruction must be `ENTRY`; the
emulator reports the pending `PS.CALLINC` and window depth when that prologue
is absent. Direct ELF loading remains the intentionally weaker debugging mode
with synthetic direct state.

The EFM8BB52 MCS-51 model includes UART1 through its documented SFR page
`0x20`. The functional slice supports baud-generator enable, transmit capture,
bounded host receive injection, SCON1 status, FIFO count/status aliases, and
the native `0x007b` interrupt vector with automatic SFR-page save/restore.
FIFO threshold timing, LIN, CTS/RTS, parity framing, and exact historical 8051
timing remain outside this slice.

Timer3, Timer4, and Timer5 provide functional 16-bit system-clock, reload,
low/high overflow, enable/priority, and interrupt paths through their page-0
and page-`0x10` registers. Their native vectors are `0x0073`, `0x008b`, and
`0x0093`; split, capture, external-clock, and cycle-accurate timing modes remain
outside this slice.

ADC0 accepts deterministic host-provided multiplexer channel codes, supports
software-triggered 8-, 10-, and 12-bit formatting with repeat/shift controls,
latches end-of-conversion and window-comparison flags, routes native vectors
`0x004b` and `0x0053`, and exposes result/flag signals in VCD. Autoscan and
timer triggers, reference/gain physics, and calibration remain outside this
functional model.

DAC0 exposes its page-`0x30` register path and latches right- or left-justified
10-bit input data with the documented low-before-high update inhibit. Enable
state and digital output code are observable through VCD and signal-stop
workflows. Timer/CLU triggers, warm-up timing, and reference/gain voltage
physics remain outside this model.

CMP0 and CMP1 compare deterministic host-controlled positive/negative codes,
honor output inversion, latch rising/falling flags, route their enable and
priority bits to vectors `0x0063` and `0x006b`, and expose both outputs in VCD.
Voltage, hysteresis, response-time, reference-DAC, and synchronization physics
are intentionally not claimed.

CLU0-3 implement the documented three-input LUT, external-pin selection,
LUT/D-flip-flop output selection, CLOUT0 readback, rising/falling edge flags,
the CL0 enable/priority interrupt path, and VCD outputs. Host overrides provide
deterministic values for internal sources not otherwise modeled. Clock-source
timing, complete internal peripheral routing, and electrical synchronization
remain outside this functional slice.

P0, P1, and P2 mask/match registers compare resolved input pins, expose a
stable `board.efm8bb52f32g.port_match.event` signal, and route mismatches
through EIE1/EIP1/EIP1H to vector `0x0043`. The event is a deterministic level
while a masked input differs; wake-state flags and electrical synchronization
are not modeled.

The 32 KiB EFM8 code-flash model accepts firmware MOVX program and page-erase
operations after the documented `FLKEY=0xa5`, `FLKEY=0xf1` sequence and
`PSCTL.PSWE` enable. Programming has NOR semantics (bits only change from one
to zero), while `PSCTL.PSEE` erases the addressed 2 KiB page. Each operation
consumes the key sequence; missing or invalid authorization leaves flash
unchanged and latches `PSCTL.PERRF`. Image loading remains a separate debugger
operation and does not weaken the firmware-visible write controls. Programming
voltage physics, timing, endurance, lock-byte policy, and hardware debug access
are outside this deterministic functional model.

The EFM8 priority crossbar gives UART0 its fixed P0.4/P0.5 routes, excludes
pins selected by `P0SKIP` through `P2SKIP`, and assigns enabled XBR0/XBR1/XBR2
resources to the remaining P0-P2 pins in priority order. Typed accessors and
VCD signals expose the selected routes and global driver-enable state.
Peripheral waveform generation remains the responsibility of each peripheral
model; selecting a route alone does not claim electrical or timing fidelity.

The EFM8 clock/power slice names and masks `CLKSEL`, `CLKGRP0`, `HFO0CN`,
`LFO0CN`, `PCON0`, `PCON1`, `REG0CN`, and `PSTAT0`. It reports the selected
source, divider, and nominal SYSCLK, accepts an explicit host frequency for
EXTOSC, and exposes clock and power state in VCD. IDLE and SNOOZE can wake from
an enabled interrupt or an explicit host request; STOP and SHUTDOWN require
reset. Oscillator settling, missing-clock detection, external pin waveforms,
regulator physics, peripheral clock domains, and exact low-power timing are
outside the deterministic functional boundary.

## Timing and tracing

One completed instruction or architectural action advances one abstract tick.
Timers, PIO and external pin stimuli use that deterministic timeline. The
model is not tied to target clock frequency. PIO uses a deterministic 16.8
divider accumulator and instruction delay fields to decide when a state
machine advances, but this remains abstract scheduling rather than a claim of
silicon clock-edge accuracy. VCD uses one nanosecond per abstract tick as a
display convention, not a hardware timing claim.

The ESP32-S3 eFuse slice retains the native staging, read-data, error, timing,
command, status, and interrupt registers. `Esp32S3EfuseRegister` names every
implemented native register and applies the official read/write masks, reset
defaults, reserved-hole rejection, strict aligned 32-bit access, and
read-only/write-only/W1C semantics. It provides deterministic factory identity
words, read-only OTP views, one-way bitwise programming for the documented
blocks, including the native `BLOCK0` staging split between `PGM_DATA0` and
`PGM_DATA1..5`. Programming and read command strobes require the documented
`0x5A5A`/`0x5AA5` opcodes, clear staging registers after programming, redact
key blocks through `RD_DIS`, and expose read/program completion interrupts. It
intentionally does not model programming voltage, Reed-Solomon correction
timing, secure-boot policy, or physical-fuse failure characteristics.

Signals use `0`, `1`, high impedance, and unknown/contention states. Changes
are streamed, and declaration/change digests are stable for equivalent runs.
The CLI accepts scheduled input in `PIN=VALUE@TICK` form.

The ESP32-S3 RMT slice maps transmitter channels 0–3 and receiver channels 4–7
at the native `0x6001_6000` block. Firmware can write the APB FIFO item format,
start a transmit channel, receive a completion interrupt, and observe the
deterministic pulse stream at `board.esp32s3.rmt.ch0` through `ch3` in VCD.
Bounded host-injected receive pulses populate the native receive memory and
interrupt state. Carrier modulation, DMA and source-clock fidelity remain
outside this functional model.

## MSP430FR2433 RTC counter slice

The MSP430FR2433 counter-only RTC is mapped at its native `0x0300` window:
`RTCCTL` (`0x00`), `RTCIV` (`0x04`), `RTCMOD` (`0x08`), and `RTCCNT`
(`0x0c`). Selecting a documented `RTCSS` source starts deterministic abstract
time; `RTCPS` selects the TI predivider set (1, 10, 100, 1000, 16, 64, 256,
or 1024). Reaching the modulo value sets `RTCIF`, and `RTCIE` routes the
overflow to vector `0xffe8`. Reading `RTCIV` returns the overflow code and
clears the flag. `RTCSR` resets the count and restarts the modulo epoch.

The observable request is `board.msp430fr2433.rtc.irq`. This model intentionally
does not claim calendar, alarm, crystal, low-power electrical, or exact clock
fidelity; the part's documented RTC is a counter, and the emulator expresses
it on the same deterministic abstract timeline as the other peripherals. The
register map and behavior are based on TI's [MSP430FR2433 datasheet](https://www.ti.com/lit/ds/symlink/msp430fr2433.pdf)
and [MSP430FR2xx/4xx Family User's Guide](https://www.ti.com/lit/ug/slau445/slau445.pdf).

Direct runs accept repeatable `--breakpoint ADDRESS` and `--watchpoint ADDRESS`
controls plus `--stop-signal PATH=change|rising|falling`. Addresses may be
decimal or `0x`-prefixed hexadecimal. A breakpoint stops before executing the
named address. A watchpoint stops after a completed CPU data read/write that
overlaps the named byte, and records the access address and kind in JSON.
Signal stops use stable hierarchical paths and preserve the triggering change
in VCD/digest output. `scripts/qualify-stop-conditions.sh` checks every stop
class on RISC-V, Arm, and Xtensa.

### RP PIO functional baseline

RP2040 PIO0/PIO1 and RP2350 PIO0/PIO1/PIO2 are mapped at their native bases
(`0x50200000`, `0x50300000`, and `0x50400000` where present). The shared
functional model uses named `RpPioRegister` and `RpPioStateMachineRegister`
identifiers and covers instruction memory, four state machines, all eight
instruction families (`JMP`, `WAIT`, `IN`, `OUT`, `PUSH`/`PULL`, `MOV`, `IRQ`,
and `SET`), shift counters, automatic push/pull, FIFO and instruction stalls,
joined FIFO capacities, wrap, side-set, delay fields, and deterministic 16.8
divider pacing. `FSTAT`/`FLEVEL`/`FDEBUG`, internal IRQ flags, versioned
`DBG_CFGINFO`, processor IRQ masks, and TX/RX DREQ state are functional. PIO
outputs reach shared GPIO nets only through the IO-bank function mux and pad
output-enable path; input sampling follows the corresponding overrides and pad
input-enable setting. RP2350 additionally implements IRQ1 placement, GPIOBASE
for GPIO32-47, and the processor FIFO PUT/GET windows. PIO v1 state-machine
PUT/GET and cross-PIO opcode extensions, plus exact silicon cycle timing,
remain known gaps. The register layout is checked against the
official [RP2040 PIO definitions](https://github.com/raspberrypi/pico-sdk/blob/master/src/rp2040/hardware_regs/include/hardware/regs/pio.h)
and [RP2350 PIO definitions](https://github.com/raspberrypi/pico-sdk/blob/master/src/rp2350/hardware_regs/include/hardware/regs/pio.h).

The ESP32-S3 AES accelerator is mapped at `0x6003_a000` using the native
`hwcrypto_reg.h` offsets. The functional slice supports AES-128 and AES-256
single-block encryption/decryption through the text-in/text-out window,
completion busy/interrupt state, and the VCD path
`board.esp32s3.aes.text_out`. DMA cipher modes, GCM/CTR/CBC/CFB/OFB
chaining, eFuse-backed keys, and cycle timing remain unsupported. The model
exposes a named `Esp32S3AesRegister` enum for the complete native key/text,
mode, IV/GCM, DMA, interrupt, and DATE map, applies conservative access masks,
treats trigger/continue/interrupt-clear fields as write-only strobes, and
rejects reserved holes, read-only output writes, and values wider than 32 bits.

The supported host matrix is Linux/amd64 and Linux/arm64. A pinned Rust
container runs the fake dual-core/timer scheduler test on both architectures;
each architecture checks 64 repeat and insertion-stress variants against the
same fixed digest. `scripts/qualify-host-determinism.sh` regenerates the
machine-readable evidence in `qualification/host-determinism.json`. A native
arm64 host or configured aarch64 binfmt handler is required for the arm64 lane.

## STM32L432KC USART1/LPUART1 slice

The STM32L432KC machine maps the native USART1 window at `0x4001_3800` and
LPUART1 window at `0x4000_8000`, alongside the existing USART2 window at
`0x4000_4400`. Each instance implements the common L4 `CR1`, `ISR`, `ICR`,
`RDR`, and `TDR` register offsets. Firmware writes to `TDR` are captured as
deterministic transmit bytes; host-injected bytes set `RXNE/RXFNE` and are
consumed by reading `RDR`. `TXE/TXFNF`, `TC`, and receive/transmit interrupt
enables are modeled functionally and routed to the corresponding NVIC lines
(USART1 37, USART2 38, LPUART1 70).

This is a register and byte-stream slice based on ST's
[STM32L432KB/STM32L432KC data sheet](https://www.st.com/resource/en/datasheet/DM00257205.pdf)
and [RM0394 reference manual](https://www.st.com/resource/en/reference_manual/rm0394-stm32l41xxx42xxx43xxx44xxx45xxx46xxx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf).
It does not claim baud-rate timing, DMA, alternate-function pin routing,
framing/error generation, low-power clock behavior, or the other USART/LPUART
modes.

## Compiler containment

Firmware compilation is never invoked directly on the host. The corpus runner
requires an immutable Docker digest or image ID and applies:

- `--pull=never` and `--network=none`
- a read-only container root
- dropped Linux capabilities and `no-new-privileges`
- read-only source and dedicated writable output mounts
- explicit CPU, memory, PID, and wall-time limits
- stable environment ordering and complete input/output SHA-256 provenance

The initial cross-GCC image contains Ubuntu-packaged RISC-V and Arm
bare-metal GCC. The Xtensa image verifies the SHA-256 of Espressif’s official
toolchain archive before extracting it. The Rust image pins Rust 1.97.1 and
prebuilds `core` for the otherwise undistributed exact RV32E register-ABI target while
installing the upstream RV32IMAC, Armv6-M, and Armv8-M libraries. Every actual
Rust case build remains network-isolated and read-only.

The ATmega328PB functional slice includes both native TWI register banks
(`TWBRn`, `TWSRn`, `TWARn`, `TWDRn`, `TWCRn`, and `TWAMRn`) at
`0xb8..0xbd` and `0xd8..0xdd`, with independent deterministic START, transmit,
receive, status, and interrupt behavior. Register reset values,
write-one-to-clear control semantics, reserved-bit handling, and the TWWC data
collision flag follow Microchip's [TWI control-register](https://onlinedocs.microchip.com/oxy/GUID-0EC909F9-8FB7-46B2-BF4B-05290662B5C3-en-US-12.1.1/GUID-1E9DD1D3-4D52-4B17-979C-13B5AA4AC1A1.html),
[status-register](https://onlinedocs.microchip.com/oxy/GUID-0EC909F9-8FB7-46B2-BF4B-05290662B5C3-en-US-12.1.1/GUID-AE5E72CE-344A-4C37-8F5B-9948EB814739.html),
and [data-register](https://onlinedocs.microchip.com/oxy/GUID-0EC909F9-8FB7-46B2-BF4B-05290662B5C3-en-US-12.1.1/GUID-6EAB15A1-6D6A-4723-A787-E8275BE8A49E.html)
descriptions. It exposes independent host byte queues for tests, but does not
claim electrical I²C arbitration, clock stretching, multi-master timing, or pin
multiplexing.

## ATmega328PB explicit fidelity boundaries

The PTC is not a conventional software-owned register peripheral: Microchip's
QTouch library owns its acquisition sequencing and depends on analog charge
transfer, calibration, sensor geometry, and timing. Renvo therefore leaves PTC
EOC/window-comparator behavior explicitly unsupported instead of fabricating
register storage that could make a custom driver appear valid. Firmware can
still exercise GPIO and ADC-based touch algorithms using deterministic digital
inputs and ADC samples.

Self-programming is likewise bounded deliberately. Normal flash execution,
boot-vector placement, EEPROM, and reset behavior are modeled, but `SPM` page
erase/write, boot-lock enforcement, signature-row reads, and fuse programming
are not yet persistent silicon operations. Custom bootloader or programming
drivers must treat those operations as unsupported until page-buffer, NRWW/RWW,
lock-bit, and fuse semantics have dedicated tests. These boundaries keep driver
tests fail-closed rather than silently accepting writes with no hardware effect.

`scripts/qualify-rust-abi.sh` compiles one freestanding ABI/behavior program at
`-O0`, `-O2`, and `-Os`. Its 18 deterministic proof runs cover CH32V003,
CH32V006, ESP32-C6, RP2040, RP2350 Arm, and RP2350 Hazard3. The program exercises
calls, slice iteration, structure and five-argument ABI passing, rotates,
wrapping arithmetic, and static data. `qualification/rust-abi.json` records the
ELF, build, result, source, compiler-image, and repeat-run hashes.

`scripts/docker-smoke.sh` builds and runs all six targets, both RP2350 CPU
modes, real-register GPIO cases, native-address UART cases, native WCH/RP
timer-interrupt cases, the full RP2350 Hazard3 compiler ISA gate, and the Rust
ABI matrix. It also runs the Arm SysTick/multi-bank NVIC and Cortex-M33
hard-float/DSP gates recorded in `qualification/arm-cpu.json`.
The same gate also runs the windowed Xtensa ABI, exception, atomic, FPU, memory
view, and deterministic-repeat matrix recorded in
`qualification/xtensa-cpu.json`.

The ESP32-S3 digital-signature slice follows the native page layout: a
contiguous 396-word C/Y/M/RB/BOX write-only window, four-word IV window, and
128-word X write-only and Z read-only windows, followed by the SetStart,
SetMe, SetFinish, QueryBusy, QueryKeyWrong, QueryCheck, and Date registers.
`Esp32S3DigitalSignatureRegister` enforces the documented masks, reset date,
reserved holes, aligned 32-bit accesses, and access directions. The functional
operation uses a deterministic SHA-256 baseline for protocol testing; secure
key provisioning, RSA-PSS padding and hardware timing remain outside this
qualification slice. `QueryKeyWrong` is reserved for the HMAC-to-DS key
handoff: malformed encrypted parameters set the MD check bit instead. The
model can select ready, not-activated, or failed HMAC handoff states for
deterministic tests. SetFinish clears the native data windows as documented.

The final distillation gate adds four bounded capabilities:

- `remu corpus reduce` minimizes source fragments, flags and numeric inputs,
  recompiling every predicate inside the pinned Docker toolchain. The checked
  three-family evidence is `qualification/reduction.json`.
- `remu script` evaluates Starlark assertions over caller-selected JSON only.
  Its portfolio proof is `qualification/starlark.json`.
- ESP32-C6/S3 direct runs can opt into `--agent-script`. Its `main()` receives
  an opaque live-machine capability for bounded run slices, debugger reads and
  writes, stops, deterministic input, and paginated RF evidence. Scripts may
  load workspace-confined `.star` workflow modules, and `--agent-artifact`
  records their JSON-compatible decisions plus a compact run summary. The C6
  real-ROM gate uses this surface to prove genuine HE20/iTWT timer re-arming
  and repeated wake events over 60 million instructions. The separate
  `--radio-script` callback remains machine-isolated. Neither surface offers
  symbol hooks or substitutes Starlark behavior for an LLE peripheral; see
  `docs/AGENT_STARLARK.md`.
- `remu board` resolves loaded Starlark board definitions into immutable
  topology/actions, then hands that scenario to the Rust board runner. It does
  not expose live CPU, scheduler, or peripheral state to the Starlark VM. With
  `--elf`, the M5StickS3 scenario binds native ESP32-S3 GPIO, SPI3, I2C1,
  I2S0/I2S1 and RMT activity to the complete published non-radio board graph:
  ST7789, M5PM1, BMI270, ES8311 audio, buttons, infrared, Grove and Hat2. See
  `docs/BOARD_SIMULATION.md` and `scripts/qualify-board-models.sh`.
- `remu gdb` serves one GDB remote-protocol session with registers, memory,
  breakpoints, step and continue. Direct runs additionally accept `--coverage`
  and `--replay`. RISC-V, Arm and Xtensa evidence is in
  `qualification/debug-observability.json`.
- `scripts/generate-dashboard.sh` fails unless all six target manifests,
  register manifests and Phase 5 artifacts pass, then regenerates
  `qualification/dashboard.html` and `.json`.

## Unmodified vendor sample gate

`scripts/qualify-vendor-samples.sh` fetches three official files at immutable
Git commits and rejects any SHA-256 mismatch: WCH EVT `GPIO_Toggle`, Raspberry
Pi `blink_simple`, and ESP-IDF `hello_world`. The upstream C files are not
patched. Tracked startup and narrow SDK compatibility adapters provide the
direct-ELF boundary and native WCH GPIO/USART, RP SIO GPIO, or ESP UART MMIO.

The samples compile only inside the pinned Docker images and pass on
CH32V003, CH32V006, RP2040, RP2350 Arm, RP2350 Hazard3, ESP32-S3 and ESP32-C6.
`qualification/vendor-samples.json` records repositories, commits, file paths,
source hashes, build/run hashes and licence treatment. WCH source is fetched on
demand and not redistributed because its file notice restricts its use;
Pico's sample is BSD-3-Clause and the ESP-IDF sample is CC0-1.0.

## RP2040 Pico SDK regression slice

`scripts/qualify-rp2040-sdk.sh` adds two byte-exact upstream Pico Examples
cases beyond the baseline blink sample:

- `uart/hello_uart/hello_uart.c` exercises UART0 transmit, SDK newline
  conversion, and GPIO0/GPIO1 function selection;
- `pwm/hello_pwm/hello_pwm.c` exercises PWM GPIO function selection, slice-0
  wrap and channel compare registers, and the enable mask.

Both cases are compiled in the pinned Docker Cortex-M0+ toolchain and run as
bounded RP2040 ELFs. The adapter supplies only the freestanding headers,
startup, and native register calls required by the unchanged examples. Bus
logs and UART output are checked against the native addresses and recorded in
`qualification/rp2040-sdk.json`. This is a narrow SDK evidence slice; it does
not imply complete UART receive, PWM timing, or general Pico SDK compatibility.

## RP2040 Pico SDK multicore regression slice

`scripts/qualify-rp2040-multicore.sh` adds the upstream
`multicore/hello_multicore/multicore.c` example. Its narrow adapter performs
the documented six-word core-1 launch handshake, drains ROM acknowledgements,
and routes FIFO push/pop traffic through the native SIO registers. The run
asserts that both cores exchange `FLAG_VALUE`, both completion messages reach
UART0, the expected character streams are present despite deterministic
per-core UART interleaving, and the primary core terminates. Evidence is in
`qualification/rp2040-multicore.json`; this remains a functional FIFO/launch
proof rather than a claim of complete multicore, spinlock, or memory-ordering
fidelity.

`corpus/edge_cases` adds 1,000 UB-free C behavioral cases with independently
generated expected values. `scripts/edge-corpus.sh` compiles them in five
Docker-only ELF layouts and runs seven CPU/target combinations. The checked
baseline passes all cases at `-O0`, `-O2`, and `-Os`.

## CoreMark qualification

`scripts/qualify-coremark.sh` compiles pinned upstream EEMBC CoreMark sources
inside the immutable Arm/RISC-V and Xtensa Docker toolchains. CH32V006,
RP2040, both RP2350 CPU modes, ESP32-S3, and ESP32-C6 pass both the standard
2,000-byte performance and validation seed sets. Results include exact CRCs,
abstract action counts, host elapsed time, ELF and run hashes, compiler
arguments, source commit, container IDs, and host identity.

CH32V003 cannot safely run the standard dataset in its 2 KiB SRAM: static data
uses 2,032 bytes and leaves 16 bytes for a stack. Its standard link is rejected
by an executable 512-byte stack-reserve assertion. It passes the distinct
upstream 1,200-byte profile-generation workload for behavioral coverage.
See [COREMARK.md](COREMARK.md) for scores and methodology.

## ATmega328PB Timer/Counter2 slice

The ATmega328PB model now exposes the Timer/Counter2 control, counter,
compare-A, interrupt-mask, and interrupt-flag registers. A deterministic
abstract-tick model supports normal overflow and CTC compare-A progression,
sets the corresponding `TIFR2` flag, resets `TCNT2`, and delivers the
enabled `TIMER2_COMPA` or `TIMER2_OVF` request on the AVR interrupt lines.
`TIFR2` uses the device's write-one-to-clear behavior, so firmware can
acknowledge a request through the native register interface.

This is a functional baseline rather than a clock-accurate timer: the
prescaler selection, asynchronous Timer2 clock, compare-B waveform output,
PWM modes, and Timer2 sleep behavior are not modeled yet. Register accesses
remain deterministic and are suitable for compiler and firmware regression
cases. The register placement and vector mapping follow the official
[ATmega328PB data sheet](https://ww1.microchip.com/downloads/en/DeviceDoc/Microchip%20AVR%20microcontroller%20ATmega328PB%20Data%20Sheet%2040001906B.pdf).

## MSP430FR2433 Timer_A slice

The MSP430FR2433 model maps all four native Timer_A blocks from TI's
peripheral map:

| Block | Base | Channels | CCR0 vector | CCR1/CCR2/TAIFG vector | VCD signal |
|---|---:|---:|---:|---:|---|
| TA0_A3 | `0x0380` | CCR0..CCR2 | `0xfff8` | `0xfff6` | `board.msp430fr2433.timer_a0.ccr0_irq` |
| TA1_A3 | `0x03c0` | CCR0..CCR2 | `0xfff4` | `0xfff2` | `board.msp430fr2433.timer_a1.irq` |
| TA2_A2 | `0x0400` | CCR0..CCR1 | `0xfff0` | `0xffee` | `board.msp430fr2433.timer_a2.irq` |
| TA3_A2 | `0x0440` | CCR0..CCR1 | `0xffec` | `0xffea` | `board.msp430fr2433.timer_a3.irq` |

Each block implements the native `TAxCTL`, `TAxCCTLn`, `TAxR`, `TAxCCRn`, and
`TAxIV` offsets. Up, continuous, and deterministic functional up/down modes
advance on abstract simulation ticks. Compare flags route CCR0 through the
dedicated vector; CCR1/CCR2 and overflow are arbitrated through `TAxIV` using
the documented priority values. Reading `TAxIV` clears the highest reported
flag, which makes ordinary MSP430 interrupt handlers usable in bounded tests.

Capture pin routing, output-unit pin multiplexing, exact clock/prescaler
fidelity, and cycle-level timer behavior remain deferred. The addresses and
vector assignments come from TI's [MSP430FR2433 datasheet](https://www.ti.com/lit/ds/symlink/msp430fr2433.pdf)
and [MSP430FR2xx/4xx Family User's Guide](https://www.ti.com/lit/ug/slau445/slau445.pdf).

## MSP430FR2433 CRC16 slice

The MSP430FR2433 peripheral window includes the native CRC block at
`0x01c0`: `CRC16DI`, `CRCDIRB`, `CRCINIRES`, and `CRCRESR`. Firmware can seed
the signature, feed 8- or 16-bit data, select the documented per-byte
bit-reversed input path, and read either the normal or bit-reversed result.
The functional model uses the MSP430 CRC-16 polynomial (`0x1021`) and keeps
all updates deterministic; the module has no interrupt or timing behavior.

Register placement and the data/reversal API follow TI's official
[MSP430FR2433 datasheet](https://www.ti.com/lit/ds/symlink/msp430fr2433.pdf)
and [FR2xx/4xx CRC driver documentation](https://software-dl.ti.com/msp430/msp430_public_sw/mcu/msp430/MSPWare/2_10_00_15/exports/MSPWare/2_10_00_15/driverlib/doc/MSP430FR2xx_4xx/html/group__crc__api.html).

## Research sources

Machine facts and register choices are based on the vendor sources linked from
the target manifests and `PLAN.html`, principally:

- WCH CH32V003/CH32V006 datasheets, CH32V00x reference material, QingKe V2
  processor manual, and the official
  [OpenWCH CH32V003 EVT sources](https://github.com/openwch/ch32v003)
- Raspberry Pi RP2040 and RP2350 datasheets and the official
  [Pico SDK PIO register definitions](https://github.com/raspberrypi/pico-sdk/blob/master/src/rp2040/hardware_regs/include/hardware/regs/pio.h)
- Espressif ESP32-S3 and ESP32-C6 datasheets and technical reference manuals,
  including the official [ESP32-S3 RMT register definitions](https://raw.githubusercontent.com/espressif/esp-idf/master/components/soc/esp32s3/register/soc/rmt_reg.h)
- Espressif’s official tool package index and crosstool-NG releases

Register behavior not covered by a passing firmware proof remains either
unmapped or explicitly approximate.

The ATmega328PB model includes both native USART0 and USART1 transmit data
registers (`UDR0` at `0xc6` and `UDR1` at `0xce`), with native USART1
enable/ready/complete status and separate trace signals. Receive, baud-rate
timing, and modem-control fidelity remain outside this functional CI slice.

## STM32L432KC SPI1/SPI3 slice

The STM32L432KC map includes native SPI1 at `0x4001_3000` and SPI3 at
`0x4000_3c00`. The functional slice covers CR1/CR2/SR/DR, enabled master-mode
full-duplex transfers, deterministic injected-or-echoed MISO, TXE/RXNE status,
and the SPI1 (35) and SPI3 (51) interrupt lines. Alternate-function pin routing,
chip-select policy, DMA, CRC, I2S, and exact clock/baud timing remain deferred.

The generated per-chip register evidence lives in
`qualification/register-coverage/`. `scripts/docker-smoke.sh` records complete
bus logs, verifies the portfolio, and regenerates all six manifests. The
manifests list observed register addresses and access kinds, proof hashes, and
known functional deviations; an unlisted address is not implicitly claimed as
either supported or unsupported.

The RP2040 and RP2350 ADC blocks are mapped at `0x4004c000` and `0x400a0000`.
They implement the named native register map, deterministic channel selection,
host-provided 12-bit samples, temperature-sensor enable, ready/result,
round-robin selection, an eight-entry FIFO, divider, FIFO-level interrupt
status, and the optional per-sample conversion-error flag in `FIFO[15]`.
The five-channel package model covers RP2040 and RP2350 QFN-60; the device
crate also exposes a nine-channel RP2350 QFN-80 variant. Analog noise,
conversion latency, calibration trim, DMA pacing, and package pin coupling are
intentionally outside this functional CI slice. The register contract is based
on the official [RP2040 datasheet](https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf)
and [RP2350 datasheet](https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf).

Direct-run `--bus-log` output is streamed as an ordered JSON array, so its
memory use is bounded independently of the number of accesses. The schema and
pretty-printed ordering remain compatible with existing qualification
artifacts. Repeated `--bus-log-region NAME` arguments retain only accesses whose
exact bus-region name matches, which keeps long vendor-firmware MMIO evidence
compact without changing execution. Coverage uses the same event stream but is
not affected by that filter and retains all unique executed addresses.
Each RISC-V CPU access includes the causative instruction `pc`. Accesses made
by autonomous peripherals, DMA helpers, debuggers, or host orchestration omit
that field rather than inheriting a stale PC. Direct-memory writes additionally
include safe `pre_value` and `post_value` fields. Device writes omit them unless
the model opts into a side-effect-free internal snapshot; the tracer never
issues a second, potentially side-effecting MMIO read to discover an old value.
The proven C6 RF/PHY register banks provide that snapshot. These fields are
observational and cannot affect bus or device behavior.
`--interrupt-log PATH` independently streams source-level transitions as
timestamped JSON. C6 and S3 radio sources use their native interrupt-matrix
numbers; only level changes are emitted, and autonomous transitions omit `pc`
instead of inheriting the preceding instruction. The exact qualified source,
W1C, reset, and unresolved-private-source boundary is documented in
[`esp-radio-interrupts.md`](esp-radio-interrupts.md) and enforced by
`qualification/radio/interrupt-contract.json`.

The ATSAMD21E18 slice also maps the native DAC at `0x42004800`. Its
functional boundary follows the typed register map from Microchip
DS40001882H §35: enable/reset, reference and event-control register state,
interrupt enable/flag semantics, 10-bit right/left-adjusted `DATA`/`DATABUF`
latches, buffered start/EMPTY/UNDERRUN behavior, IRQ 25, and deterministic
host/VCD output-code observation. Analog settling, voltage/reference accuracy,
clock synchronization, Event System/DMAC coupling, and the physical output
buffer remain outside the model.

The ATSAMD21E18 timer topology is mapped at the package's native addresses:
RTC at `0x40001400`, TCC0–2 at `0x42002000`–`0x42002800`, TC3–5 at
`0x42002c00`–`0x42003400`, and SERCOM0–3 at
`0x42000800`–`0x42001400`. RTC MODE0 provides deterministic 32-bit
count/compare/overflow behavior on IRQ 3. TCC0–2 provide 24-bit
period/compare, buffered register storage, match/overflow W1C interrupts on
IRQs 15–17, package-appropriate channel masks, and VCD-visible digital PWM
levels. TC4/5 and SERCOM1–3 reuse their audited instance-generic functional
models with the vendor IRQs. RTC calendar modes, TCC capture/fault/dead-time
and exact clock/prescaler timing remain outside this support boundary.

The STM32L432KC model includes the deterministic RNG at `0x50060800`. It
models native enable/status/data registers with a replayable host-seeded stream
for CI; this is an observability-friendly deterministic model, not a
cryptographic entropy source or silicon noise model.

For ESP32-C6, direct ELF loading proves instruction and peripheral behavior but
does not exercise the second-stage bootloader's flash mappings. Supplying the
corresponding esptool application binary with `--esp-app-image` enables a
separate boot-layout gate. It checks the chip and entry metadata, descriptor
and text segment ordering, 64 KiB mapping congruence, and correspondence with
the executable ELF. The default application partition offset is `0x10000` and
can be changed with `--esp-app-offset`. This mode requires `--boot-rom` with the
matching real mask-ROM ELF. ESP32-C6/S3 native firmware boot and radio-capable
direct execution have the same requirement; a CPU/compiler-only direct ELF
harness remains exempt.

In direct C6 runs, a main-watchdog system reset reloads the initialized ELF
segments and entry point to model the second-stage application handoff. This
lets reset/recovery firmware run across multiple boots without claiming ROM
instruction-level fidelity.

ESP32-C6 application RAM powers on with the deterministic nonzero byte pattern
`0xa5`. Direct ELF loading copies only the file-backed portion of writable load
segments, leaving each `p_memsz - p_filesz` tail poisoned. Firmware must
therefore perform its own `.bss` initialization, as it must on hardware. Other
targets retain their existing reset-memory policy.

The native-image boundary covers every target advertised by `remu targets`.
RP targets consume UF2 and preserve their Arm/RISC-V boot selection; Espressif
targets consume validated merged flash images (or the existing application-UF2
overlay path); PIC16 and EFM8 consume Intel HEX; the remaining byte-addressed
MCUs consume Intel HEX or addressless raw binaries rooted at their documented
primary flash base. Native loading uses vector/reset semantics where the
architecture defines them instead of inventing an ELF entry point.

The EFM8BB52F32G functional slice now includes Timer1 in addition to Timer0
and Timer2: mode-1/2 counter progression, overflow flagging, VCD visibility,
and a dedicated low/high interrupt line pair mapped to the native `0x001b`
vector. Timer1 mode 0/3, exact oscillator-divider timing, and the remaining
EFM8 timer blocks remain outside this slice. The register and interrupt
semantics are based on the [Silicon Labs EFM8BB52 reference
manual](https://www.silabs.com/documents/public/reference-manuals/efm8bb52-rm.pdf).

SMBus0 models leader start/stop, guest transmit capture, deterministic
follower receive injection, FIFO status/flush behavior, bus ownership, and
enabled low/high-priority service requests. Arbitration between multiple
modeled bus participants, line-level clock stretching, timeout timing, and
electrical bus behavior remain outside this functional slice.

Native/direct equivalence is continuously checked by
`scripts/qualify-native-images.sh`. All compiler inputs are built in immutable,
network-disabled Docker toolchains. The gate compares stop reason, exit code,
UART/USB output, trace digest, and byte-identical VCD for all 14 target modes.
PIC16 and EFM8 use their direct Intel HEX boundary because their toolchains do
not emit a runnable ELF; every other mode compares native boot with ELF.

The ATmega328PB slice includes a deterministic 10-bit ADC path at the native
`ADCSRA`/`ADMUX`/`ADCL`/`ADCH` registers. Tests can inject per-channel samples,
select right- or left-adjusted results, and observe the conversion-complete
flag/interrupt (vector 22 at program address `0x002a`). The functional timing
uses the documented 13/25-conversion-cycle shape and ADPS prescaler without
claiming clock-level fidelity. Auto-trigger sources, reference-voltage and
analog electrical behavior, and the temperature/bandgap ADC multiplexer inputs
remain deferred. The register behavior is based on the [Microchip ATmega328PB
ADC reference](https://onlinedocs.microchip.com/oxy/GUID-0EC909F9-8FB7-46B2-BF4B-05290662B5C3-en-US-12.1.1/GUID-AD160391-6A79-47DE-A064-2E438B4A9AA3.html).

The human-readable portfolio view is
[`qualification/dashboard.html`](../qualification/dashboard.html), with the
same checked data in `qualification/dashboard.json`. “Baseline proven” there
means a deterministic functional compiler/firmware model, never complete
silicon compatibility or cycle accuracy.

The EFM8BB52F32G slice also models the native CRC0 data path: CCITT-16 stream
updates through `CRC0IN`, pointer-selected result reads/writes through
`CRC0DAT`, CRC seed initialization, and byte bit reversal through `CRC0FLIP`.
Register masks and pointer behavior follow Silicon Labs' [EFM8BB52 reference
manual](https://www.silabs.com/documents/public/reference-manuals/efm8bb52-rm.pdf),
section 18. Automatic flash-sector CRC and the remaining analog/control blocks
are outside this functional boundary.

The STM32L432KC slice includes the native independent-watchdog window at
`0x40003000`: key unlock/start/reload commands, prescaler and reload state,
deterministic timeout reset requests, and integration with the Arm machine's
watchdog reset path. The key values, prescaler encoding, reload width, and
register offsets follow ST's [STM32L41/42/43/44 reference manual](https://www.st.com/resource/en/reference_manual/dm00151940-stm32l4x1-advanced-arm-based-32-bit-mcus-stmicroelectronics.pdf).
Window watchdog timing and low-power clock behavior remain outside this
functional boundary.

The STM32L432KC model includes ADC1 at its native AHB2 address `0x50040000`.
It supports the native enable/calibration/start sequence, regular-rank channel
selection, deterministic 12-bit host-injected samples, end-of-conversion flags,
and data-register reads. Analog settling, calibration curves, injected groups,
DMA, oversampling, and exact conversion timing remain deferred.

The STM32L432KC CRC slice maps the native data, independent-data, control,
initial-value, and polynomial registers. It supports deterministic word feeds
and exposes the current result for host-side qualification; DMA feeding and
cycle-level AHB timing remain deferred.

The STM32L432KC RTC slice maps the native calendar, prescaler, alarm, and
write-protection registers. It advances deterministically on the abstract
timeline and exposes alarm flags for qualification; oscillator drift, backup
domain power behavior, tamper inputs, and subsecond fidelity remain deferred.
