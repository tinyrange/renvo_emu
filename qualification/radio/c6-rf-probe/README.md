# ESP32-C6 RF probe

This is project-owned, freestanding low-level firmware for the ESP32-C6 RF,
PHY, MAC, and DMA contract. It includes no ESP-IDF headers and links no
Espressif runtime or radio library. The recovered RF tables are factual MMIO
outputs of the pinned public-API oracle described in `../RADIO_PLAN.html`.

`scripts/qualify-c6-rf-probe.sh` builds the firmware, creates one ESP flash
image, boots that exact image through Renvo's genuine-ROM path twice, checks
determinism, checks channels 1/6/11 and powers 8/14/20 dBm, and proves both
on-channel receive, wrong-channel rejection, and a deterministic project-owned
open-system station exchange through scan, authentication, association, and
bidirectional L2. It then runs a second project-owned station against the
deterministic WPA2-PSK peer, validates the M1/M3 transcript, derives the PTK,
installs the temporal key into the recovered C6 hardware table, and proves
authenticated CCMP transmit and receive. Both station runs repeat byte-for-byte.
Generated evidence is written
below `.remu/qualification/c6-rf-probe`.

The WPA2 fixture uses SSID `REMU-C6-WPA2` and passphrase
`renvo-c6-wpa2`. `requirements.json` pins the standard 4096-round
PBKDF2-HMAC-SHA1 result, and the qualification script re-derives it
independently before executing the image. The firmware implements SHA-1,
HMAC-SHA1, PBKDF2, the WPA pairwise expansion and EAPOL MIC handling itself;
the pinned PMK keeps the bounded exact-image run focused on the causal EAPOL
and C6 hardware-CCMP path.

The image also exposes a newline-delimited UART0 protocol at 115200 baud:
`INIT`, `CHANNEL n`, `POWER n`, `RX START`, `RX STOP`, `TX tag`, and
`RESET RADIO`. Every operation emits a stable `REMU_C6_RF` checkpoint.

This qualification is intentionally software-emulator-only. The recovered
low-level RF sequences have not been validated for radiated operation, output
power, spectral mask, harmonics, EVM, or per-device calibration. The repository
therefore provides no script that flashes or executes this probe on physical
radio hardware, and emulator evidence must not be represented as a physical RF
pass.

## Audited driver contract

`mmio-contract.json` is the complete dependency and failure-semantics manifest
for this independent driver. All 24 peripheral dependencies are centralized in
`hal/c6/registers.h`; 18 are radio registers and one is a bounded ten-word
crypto region. The remaining entries are interrupt-controller and UART
platform dependencies. Each entry records its exact access direction,
lifecycle, evidence tier, constraints, side effects, provenance, and failure
mode.

`scripts/check-c6-custom-driver-contract.py` rejects raw peripheral addresses
outside that header, undeclared or unused symbols, overlapping/unbounded
regions, read/write expansion, trace-only unknown radio registers, false
`trace-causal` claims, and anonymous numeric negative returns in the HAL. See
[`docs/custom-radio-driver-contract.md`](../../../docs/custom-radio-driver-contract.md)
for the extension and review rules.

The public HAL result codes are stable and subsystem-specific:

- RF: invalid channel, invalid power, or not initialized;
- MAC: not ready, timeout, invalid TX frame, invalid RX descriptor, or output
  buffer too small;
- PHY: reset did not acknowledge ready; and
- UART: no input byte is currently available.

Run the static gate before building the exact image:

```sh
scripts/check-c6-custom-driver-contract.py
REMU_BIN=target/release/remu scripts/qualify-c6-rf-probe.sh
```
