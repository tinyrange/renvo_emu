# M5Stack NanoC6 hardware oracle

This ESP-IDF application runs one case from each of the 40 portable corpus
kernels on a physical M5Stack NanoC6. It publishes exact 32-bit results in RAM
for a JTAG read and also emits them over the ESP32-C6 USB Serial/JTAG console.

The workflow uses the immutable official ESP-IDF v5.3.5 image
`sha256:114a9f8cde8bdc5a7e95809745b492663f7c5afe9c2821a4127ff133754fafd2`
and its Espressif GCC 13.2.0 toolchain. A small derived container pins esptool
5.3.0 for its more reliable ESP32-C6 RAM loader. It does not burn eFuses,
enable secure boot, enable flash encryption, or change write protection.

The default test is non-persistent: it builds a RAM-only application, loads it
through the built-in JTAG interface, reads the result array back through JTAG,
and compares it with the corpus manifest:

```sh
scripts/nanoc6-hardware.sh test
```

The optional `backup`, `build`, `flash`, and `capture` commands exercise the
flash/serial path. `backup` reads all 4 MiB into
`.renvo/nanoc6/factory-flash.bin` before a first flash. Flashing replaces only
the ordinary bootloader, partition table, and application regions and is
recoverable with that image or a new ESP-IDF application.

Measured on an ESP32-C6FH4 revision 0.2 NanoC6 on 2026-07-31, all 40 cases
matched both the manifest and Renvo's ESP32-C6 interpreter. The sample covers
one case from each corpus kernel; it validates RV32 compiler/CPU semantics, not
the complete ESP32-C6 peripheral model or timing.

GPIO12/13 remain assigned to native USB. The NanoC6 RGB, IR, button, blue LED,
and Grove pins are not touched. No command burns eFuses, enables secure boot or
flash encryption, or changes write protection.
