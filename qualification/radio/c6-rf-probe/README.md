# ESP32-C6 RF probe

This is project-owned, freestanding low-level firmware for the ESP32-C6 RF,
PHY, MAC, and DMA contract. It includes no ESP-IDF headers and links no
Espressif runtime or radio library. The recovered RF tables are factual MMIO
outputs of the pinned public-API oracle described in `../RADIO_PLAN.html`.

`scripts/qualify-c6-rf-probe.sh` builds the firmware, creates one ESP flash
image, boots that exact image through Renvo's genuine-ROM path twice, checks
determinism, checks channels 1/6/11 and powers 8/14/20 dBm, and proves both
on-channel receive and wrong-channel rejection. Generated evidence is written
below `.remu/qualification/c6-rf-probe`.

The image also exposes a newline-delimited UART0 protocol at 115200 baud:
`INIT`, `CHANNEL n`, `POWER n`, `RX START`, `RX STOP`, `TX tag`, and
`RESET RADIO`. Every operation emits a stable `REMU_C6_RF` checkpoint.

Run `scripts/qualify-c6-rf-probe-hardware.sh /dev/ttyACM0` to flash the same
generated image to a physical ESP32-C6 and capture its checkpoints. A hardware
pass is never inferred from an emulator run; the hardware evidence file is
created only after a real serial device has been opened and the required
checkpoints have been observed.
