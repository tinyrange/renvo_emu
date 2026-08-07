# ATSAMD21E18 qualification

The pinned Arm GNU 13.2.Rel1 and Clang/LLD 18 lanes select Cortex-M0+ Thumb code and the exact
SAMD21E18A device identity. The compiler smoke covers startup, Arm EABI calls,
native widths, arithmetic, GPIO input/output, EIC routing, TC3 interrupt entry,
SERCOM0 UART output, and the native DAC data path. It boots from the vector table at flash address zero,
uses the 256 KiB flash and 32 KiB SRAM maps, and emits `SAMD21\n`.

The functional peripheral surface is PM, SYSCTRL, GCLK and NVMCTRL startup
state, PORT A, EIC, TC3, SERCOM0 USART, watchdog, and the DAC at `0x42004800`.
The DAC follows the native `CTRLA`, `CTRLB`, `EVCTRL`, interrupt, `DATA`, and
`DATABUF` offsets, including 10-bit right/left adjustment, buffered start,
EMPTY/UNDERRUN flags, and IRQ 25. Its VCD `output_code` is a deterministic
10-bit digital observation rather than an analog voltage. Clock synchronization
and timing remain functional approximations; analog settling/reference voltage,
USB, DMA, and the QTouch PTC are unsupported.
VCD exposes PORT, timer, UART, DAC and interrupt hierarchy and the gate compares
two runs byte-for-byte.

The DAC register map and bit definitions are sourced from Microchip
DS40001882H, section 35:
<https://ww1.microchip.com/downloads/en/DeviceDoc/SAM-D21-DA1-Family-Data-Sheet-DS40001882H.pdf>.

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
