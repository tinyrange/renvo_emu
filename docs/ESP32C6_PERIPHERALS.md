# ESP32-C6 peripheral support

The ESP32-C6 machine provides a deterministic functional model, not a
cycle-accurate replacement for the silicon. This inventory is based on the
ESP-IDF v6.0.2 ESP32-C6 register headers. A mapped register facade is called
out explicitly and is not counted as functional peripheral support.

## Functional inventory

| Family | Implemented behavior | Deliberate limits |
|---|---|---|
| GPIO and IO MUX | 31 digital pads, input/output, matrix-facing signals, pad selection and VCD | Analog pad physics and electrical timing |
| UART and UHCI | UART0, UART1 and LP UART host streams; UHCI escape and quick-send routing | Baud-rate timing and physical line sampling |
| SPI and I2C | SPI2 full-duplex word transfers; I2C0 command/FIFO transactions; LP I2C transactions | Electrical arbitration, pad ownership and clock stretching |
| I2S and PARLIO | Deterministic transmit/receive sample streams and completion interrupts | Exact clock domains and physical skew |
| LEDC, MCPWM and RMT | PWM/motor timing outputs, RMT pulse streams and interrupt state | Carrier modulation, dead-time edge physics and cycle accuracy |
| PCNT | Routed edge counting, limits and interrupt state | Metastability and glitch-filter timing |
| TWAI0 and TWAI1 | Frame transmit/receive queues and controller interrupt state | Arbitration, error confinement and physical bus contention |
| SDIO slave | HINF/SLC function queues, packet interrupts and host exchange | SD electrical timing and a native SD/MMC host, which ESP32-C6 does not contain |
| Timers | Both timer groups, system timer comparators, staged MWDT/LP watchdog actions, reset scope and reset-cause reporting | Cycle-accurate clock drift |
| GDMA and ETM | Descriptor-facing transfers and observable event/task routing | Concurrent bus arbitration and cycle timing |
| Analog | SAR ADC conversion values and temperature-sensor input | Calibration drift, noise and analog settling |
| Security | AES, SHA, RSA, ECC, HMAC, digital signature and one-way eFuse programming | Secure-boot policy, flash-encryption policy, protected key provisioning and physical fuse failures |
| Startup interfaces | SPI0/SPI1 flash memory, coherent cache maintenance, USB Serial/JTAG including CONF0/TEST raw-PHY lines and transition capture, PCR startup and analog-I2C startup behavior | USB electrical timing, analog PHY behavior and bus arbitration |
| CPU-local interrupts | Machine/user PLIC contexts, machine/user CLINT software and timer interrupts, interrupt delegation | Cycle-accurate arbitration |
| Low-power domain | RV32IMAC LP core, retained LP SRAM, PMU sleep/wake transitions, LP timer wakeups and LP-AON reset controls | Analog power-transition timing |
| Memory protection | TOR, NA4 and NAPOT PMP permission enforcement, lock semantics and instruction/load/store access faults | Physical memory attributes beyond PMP |

Dedicated GPIO is represented by the GPIO block and CPU instruction behavior;
LCD output uses the chip's SPI/PARLIO paths; SD cards use SPI2; and asynchronous
memory copy uses GDMA. ESP-IDF does not expose separate public MMIO register
blocks for those API-level names.

## Register facades

PMU/LP-AON/LP timer, EXTMEM cache control, and machine/user PLIC and CLINT now
have dedicated functional models. Atomic, SLCHOST, PVT/memory monitor, PAU, HP
system, PCR, TEE/APM, miscellaneous system, power detector, LP clock/reset, LP
IO, LP analog, LP protection, trace and assist-debug remain strict register
facades. These remaining blocks reject undocumented offsets and preserve normal
word/halfword/byte register semantics; trace capture, analog behavior and the
full monitor/APM policy engines are not modeled.

## Radio boundary

The IEEE 802.15.4 page and the Wi-Fi/Bluetooth modem pages are intentionally
unmapped. Radio behavior will use one shared Wi-Fi 6, Bluetooth LE and IEEE
802.15.4 design instead of merging the isolated frame-loopback prototype.

## Qualification

`scripts/qualify-esp32c6-peripherals.sh` downloads an exact, SHA-256-verified
set of ESP-IDF v6.0.2 register headers, compiles a bare-metal probe with the
pinned official `riscv32-esp-elf` 14.2.0 toolchain, and runs it on the ESP32-C6
machine. The probe checks native reset identities, configuration semantics,
SPI transfer behavior, PLIC/CLINT delivery, PMU/LP-timer state, cache sync,
PMP configuration and bus-log coverage across every functional family. Rust
unit and machine tests additionally cover data paths, interrupts, LP-core
execution, cache coherency, staged resets, reserved accesses and the absence of
radio mappings. The raw USB-PHY tests also cover C6 pull/line-state controls and
an independent low-speed packet oracle for NRZI, bit stuffing, PID, CRC16, EOP
and per-bit instruction cadence. The recovery fixture deliberately wedges after
raw-PHY takeover and proves that an MWDT system reset retains the LP-AON marker,
boots the application again, restores USB Serial/JTAG, and reaches the guarded
recovery window.
