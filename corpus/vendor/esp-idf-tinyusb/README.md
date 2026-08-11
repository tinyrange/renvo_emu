# ESP-IDF and TinyUSB qualification adapter

This adapter runs Espressif's `tinyusb_driver_install()` and the upstream
TinyUSB device, CDC, FIFO, and Synopsys DWC2 implementations as a deterministic
ESP32-S3 ELF. `scripts/qualify-esp32s3-usb-stacks.sh` fetches and verifies the
exact upstream revisions before compiling them with this adapter.

Only the surrounding ESP-IDF services are replaced here: task creation,
interrupt registration, PHY allocation, logging, and the small C runtime.
Interrupts are polled by the probe so USB enumeration is deterministic. The
USB stack, descriptors, endpoint handling, and DWC2 register programming remain
the vendor implementations under qualification.
