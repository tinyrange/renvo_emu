# ATSAMD21E18 qualification

The pinned Arm GNU 13.2.Rel1 and Clang/LLD 18 lanes select Cortex-M0+ Thumb code and the exact
SAMD21E18A device identity. The compiler smoke covers startup, Arm EABI calls,
native widths, arithmetic, GPIO input/output, EIC routing, TC3 interrupt entry,
SERCOM0 UART output, and the USB device control/endpoint register surface. It
also configures the 12-channel EVSYS register
surface, user mux, synchronous software event, event-detected flag, and
write-one-to-clear behavior. It boots from the vector table at flash address zero,
uses the 256 KiB flash and 32 KiB SRAM maps, and emits `SAMD21\n`.

The functional peripheral surface is PM, SYSCTRL, GCLK and NVMCTRL startup
state, PORT A, EIC, TC3, SERCOM0 USART, SPI host, I²C host, EVSYS, USB, and
watchdog. SERCOM
transfers use deterministic register-level loopback/injected responses; pin
electrical timing and complete client/slave behavior are not modeled. The
register implementation follows the vendor mode encodings, per-mode masks,
enable-protection, raw interrupt aliases, I²C bus-state/command semantics, and
SPI receiver-enable behavior. Clock synchronization and timing are deterministic
approximations. Full USB packet protocol, descriptor DMA, analog behavior,
standalone DMAC, and live peripheral event-generator/user routing are
unsupported.
VCD exposes PORT, timer, UART and interrupt hierarchy and the gate compares two
runs byte-for-byte.

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
