# ATSAMD21E18 qualification

The pinned Arm GNU 13.2.Rel1 and Clang/LLD 18 lanes select Cortex-M0+ Thumb code and the exact
SAMD21E18A device identity. The compiler smoke covers startup, Arm EABI calls,
native widths, arithmetic, GPIO input/output, EIC routing, TC3 interrupt entry,
SERCOM0 UART output, and the SAM D21 USB device control/endpoint register
surface. The USB slice follows the vendor CMSIS register masks, including
FSMSTATUS, descriptor and pad-calibration fields, endpoint status aliases and
write-one-to-clear interrupt flags. The smoke also configures the 12-channel
EVSYS register surface, user mux, synchronous software event, event-detected
flag, write-one-to-clear behavior, a software-triggered DMAC descriptor
transfer, and native I2S clock/serializer/data control. It boots from the
vector table at flash address zero,
uses the 256 KiB flash and 32 KiB SRAM maps, and emits `SAMD21\n`.

The functional peripheral surface is PM, SYSCTRL, GCLK and NVMCTRL startup
state, PORT A, EIC, TC3, SERCOM0 USART, SPI host, I²C host, EVSYS, USB, I2S,
ADC, AC, DAC, and watchdog, plus the DMAC common/channel registers. The DMAC model follows the
vendor masks and direct/W1C access semantics, including the reserved gap
between DBGCTRL and SWTRIGCTRL. It executes one valid
software-triggered descriptor for memory-to-memory byte/halfword/word
transfers, records write-back state, and latches completion/fetch-error flags.
The I2S two-clock/two-serializer control, interrupt, and sample-holding
registers capture transmitted sample words and latch ready/overrun flags for
host-injected receive words. The model uses documented register masks and
routes the shared I2S interrupt to Cortex-M0+ IRQ 27. SERCOM
transfers use deterministic register-level loopback/injected responses; pin
electrical timing and complete client/slave behavior are not modeled. The
register implementation follows the vendor mode encodings, per-mode masks,
enable-protection, raw interrupt aliases, I²C bus-state/command semantics, and
SPI receiver-enable behavior. Clock synchronization and timing are deterministic
approximations. The ADC exposes its native control/reference/input/sequence
surface and deterministic host samples for software-triggered conversion,
including bounded positive-input scan advancement; result-ready, overrun, and
window flags route to IRQ 23. It follows vendor masks and raw write-one
interrupt alias semantics. Full USB packet protocol and USB descriptor DMA, linked DMAC
descriptors, peripheral/event trigger routing, CRC execution, analog behavior,
ADC averaging/event-DMA coupling, I2S serial timing/framing/pin waveforms/DMAC coupling, and live peripheral
event-generator/user routing are unsupported.
The AC exposes native control/input/status/interrupt registers, deterministic
host-supplied first-pair AIN codes, single-shot and continuous comparison,
edge/EOC flags, window state, and VCD-visible digital comparator outputs. It
follows vendor masks, raw interrupt alias semantics, CTRLA low-power bits, and
SWAP behavior. Comparator filtering, startup behavior, and exact window
electrical behavior remain unsupported.
The DAC follows the native `CTRLA`, `CTRLB`, `EVCTRL`, interrupt, `DATA`, and
`DATABUF` offsets, including 10-bit right/left adjustment, buffered start,
EMPTY/UNDERRUN flags, and IRQ 25. Its VCD `output_code` is a deterministic
10-bit digital observation rather than an analog voltage. Analog settling and
reference voltage are unsupported.
VCD exposes PORT, timer, UART, comparator, DAC, and interrupt hierarchy, and the gate compares
two runs byte-for-byte.

The DAC register map and bit definitions are sourced from Microchip
DS40001882E, section 35:
<https://ww1.microchip.com/downloads/en/DeviceDoc/SAM_D21_DA1_Family%20Data%20Sheet_DS40001882E.pdf>.

The vendor lane builds the pinned Microchip Harmony
`port_led_on_off_polling` main source unchanged. Tracked startup, declarations
and MMIO adapters bind it to the E18 package and stop on the expected PA07
edge. The source is
`apps/port/port_led_on_off_polling/firmware/src/main.c` at revision
`9331e79cf2937d2b3166813c6d2886b2481162e3`; its Microchip Software License
Agreement v2.001 device-use terms remain applicable. The expected result is a
rising PA07 edge; only startup, declarations, and host MMIO binding are local.
Run:

```sh
scripts/qualify-atsamd21e18.sh
```
