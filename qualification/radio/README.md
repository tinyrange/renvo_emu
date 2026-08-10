# ESP32-C6 and ESP32-S3 radio qualification

This directory is the auditable entry point for issue #338. Radio behavior is
host-isolated and deterministic by default; no qualification case opens a host
socket or joins a physical network.

## Reproducible local gates

```sh
python3 scripts/check-radio-sources.py
scripts/fetch-esp-rom-elfs.sh
scripts/qualify-esp-radio-rom.sh esp32c6
scripts/qualify-esp-radio-rom.sh esp32s3
scripts/qualify-esp-radio-wifi-softap-vendor.sh esp32c6
scripts/qualify-esp-radio-wifi-softap-vendor.sh esp32s3
scripts/qualify-esp-radio-coexistence-vendor.sh esp32c6
scripts/qualify-esp-radio-coexistence-vendor.sh esp32s3
scripts/qualify-esp-radio-ble-vendor.sh esp32c6
scripts/qualify-esp-radio-ble-vendor.sh esp32s3
scripts/qualify-esp-radio-ieee802154-vendor.sh esp32c6
scripts/qualify-esp-radio-openthread-vendor.sh esp32c6
scripts/qualify-esp-radio-custom-stack.sh esp32c6
scripts/qualify-esp-radio-custom-stack.sh esp32s3
cargo test -p remu-radio
cargo test -p remu-machines esp32c6_ieee802154
cargo test -p remu-machines esp32c6_wifi_and_ble
cargo test -p remu-machines esp32s3_wifi_and_ble
RUSTFLAGS=-Awarnings scripts/qualify-esp32c6-peripherals.sh
RUSTFLAGS=-Awarnings scripts/qualify-esp32s3-peripherals.sh
cargo check -p remu-web --target wasm32-wasip2 --locked
```

`source-ledger.json` records the immutable revision, digest, license, and use of
every research input. `inventory.json` records protocol applicability, public
MMIO blocks, disputed address claims, and every radio interrupt source. The
source checker fails closed for code or documentation whose license is absent,
unknown, or outside the explicit permissive allowlist.

The pinned ROM fetch installs the C6 rev0 and S3 rev0 Apache-2.0 ELF artifacts
under `.remu/qualification/esp-rom-elfs/20260528`. Renvo executes their mapped
ROM sections as guest instructions. ELF symbols may label offline disassembly,
but are discarded from the runtime image and never select emulator behavior.
`rom-requirements.json` makes both chips mandatory: CI rebuilds the
repository-owned public-API probe against unmodified ESP-IDF Wi-Fi/PHY/ROM
libraries, executes the pinned ROM and application, rejects
all faults before the required instruction budget, and verifies execution
coverage contains both the mask-ROM range and the application entry point. The
gate also requires the UART evidence emitted by the real application, Wi-Fi
library, PHY initialization, a completed one-channel active scan, and a native
open-system station association. Each chip transmits exact probe,
authentication, and association requests through native DMA, receives
deterministic beacon, authentication-response, and association-response frames
through its real RX ring, reports the AP through `esp_wifi_scan_get_ap_num()`,
then reaches `WIFI_EVENT_STA_CONNECTED` with AID 1. The gate checks each chip's
real station address and exact management-frame lengths. Hardware FCS allowance
is validated separately from guest MAC-frame capacity and excluded from replay
bytes. Every run is repeated from reset and its RF artifact must be
byte-for-byte identical. The C6 and S3 ROMs are separate required inputs and
matrix jobs: passing with one chip's ROM does not satisfy the other chip's ROM
requirement.

The complementary genuine SoftAP/ESP-NOW gate configures an open channel-one
AP through public ESP-IDF APIs, emits native beacons and a broadcast ESP-NOW
action frame, and injects authentication, association, and directed ESP-NOW
frames through the vendor-owned RX ring. C6 and S3 must select the AP virtual
interface from the MAC address slots programmed by their respective firmware,
emit native authentication and association responses, report the peer with AID
1, and deliver the exact `deadbeef` payload to the public ESP-NOW callback. The
gate uses each required real ROM, forbids runtime symbol dispatch, and requires
byte-identical RF replay from reset.

The genuine coexistence gate runs that public SoftAP workflow concurrently
with public NimBLE advertising and scanning. Every native Wi-Fi or BLE frame
must have exactly one matching chip-local coexistence grant before it reaches
the RF medium, and both the grant stream and RF artifact must replay exactly
from reset. A denied request remains an ordinary arbitration outcome and does
not transmit; submitting a frame under another protocol's grant is an illegal
emulator state and terminates with the stable hard-error diagnostic defined by
`legal-state-contract.json`. On C6, the same genuine image also enables the
public IEEE 802.15.4 driver and transmits while Wi-Fi remains active and BLE is
scanning, so all three silicon radio families cross the same ownership gate.

Vendor-independent execution is a focused low-level regression track. The two
`custom-stack-probe` ELFs are original freestanding firmware: they include no
ESP-IDF headers and link no Espressif runtime, PHY, Wi-Fi, Bluetooth, or
802.15.4 library. CI compiles them with the pinned architecture toolchains,
executes them as guest instructions, requires a zero exit code, and checks the
native radio MMIO regions in the bus trace. C6 currently proves low-level
Wi-Fi MAC TX/RX, BLE TX/RX, and 802.15.4 MAC/DMA TX/RX, multi-PAN filtering,
matching-ACK, and no-ACK timer boundaries; S3 proves its low-level
Wi-Fi MAC TX/RX, BLE link-layer TX/RX, TSF/timebase, entropy, completion, and
W1C boundaries. Both
firmware images own native receive descriptors, validate RX metadata and frame
bytes in RAM, and service the native RX completion event. The emitted
summary records the ELF digest, coverage digest, and exercised families. Each
probe is then rerun from reset: the complete run result must replay exactly and
the RF artifact must be byte-for-byte identical.

Current BLE evidence uses two complementary viewpoints on both chips. Each genuine ESP-IDF/NimBLE probe
initializes the real controller and PHY, programs native advertising and scan
activities, transmits an exact advertisement, and receives an injected
advertisement through the vendor-owned RX ring. Its public GAP callback must
return the exact advertising data and -80 dBm received power. Each freestanding
probe independently programs its chip's scheduler, descriptor/shared-memory
layout, scan window, RX descriptor and payload buffer, then validates ownership
transitions, metadata, RX/END interrupts, and W1C clearing. Each run requires
identical radio replay on reset.

C6 also has genuine ESP-IDF/NimBLE and freestanding BLE evidence for native
controller TX/RX and interrupt service. C6 IEEE 802.15.4 has complementary
probes: the public
ESP-IDF API and the freestanding MAC probe cover native TX/RX, two-interface
PAN filtering, matching ACK reception, no-ACK timeout handling, DMA metadata,
receive-side automatic ACK transmission with frame-pending state, distinct W1C event bits,
and exact replay. The vendor gate additionally proves the public multi-PAN
index, public no-ACK error mapping, and callback delivery after ACK
transmission. The focused probe's overlapping frame checks are useful
low-level regressions, not a requirement to reproduce every vendor-stack test.

The real ESP32-C6 rev0 and ESP32-S3 rev0 mask-ROM images are mandatory for
native boot, radio-capable execution, and vendor-facing qualification. Their
pinned digests are checked by qualification before execution, and
functional-ROM dispatch is disabled when they are loaded. Direct CPU/compiler
ELF harnesses that neither boot a native image nor exercise radio remain a
lightweight exception. Freestanding probes remain targeted tests of low-level
hardware contracts; they do not need to duplicate every vendor-stack behavior.

## Implemented functional boundary

- A shared packet-airtime RF medium with collision, capture, sensitivity,
  deterministic loss, path loss, explicit injection, and replay artifacts.
- A stable coexistence arbiter with grants, denial, priority, preemption, and
  append-only evidence.
- ESP32-C6 MODEM_SYSCON/MODEM_LPCON clock/reset gates and the public IEEE
  802.15.4 MAC register block, including DMA, filters, automatic ACK, FCS,
  timed single-shot CCA/ED for guest-owned replayable CSMA-CA, AES-CCM*
  transmit security with published negative reason codes, timers, W1C events,
  abort reasons, counters, and interrupt source 12. The genuine ESP-IDF gate
  covers both clear and busy CCA: it observes public error 1, clears the abort
  cause, and performs a firmware-owned successful retry.
- A shared BLE H4 controller for C6 and S3 with periodic advertising/scanning,
  initiating, ACL, encryption, data-length, PHY, privacy-facing queries,
  deterministic peers, and controller events.
- A shared Wi-Fi functional engine for C6 and S3 with deterministic peer scans,
  station association, channel/address filters, raw TX/RX, monitor mode,
  power-save state, interrupts, and reset.
- Native Rust and browser/WASI frame-injection and replay APIs. The radio-aware
  browser/WASI call requires and verifies the matching pinned real mask-ROM ELF
  before execution. External host networking remains deliberately absent.

## Gates that remain open

The presence of a functional host API is not treated as vendor-firmware
compatibility. The definition of done still requires pinned, unmodified
ESP-IDF applications to initialize through their real controller/ROM/shared
memory ABI. Wi-Fi still needs ACK/retry, fragmentation/aggregation, hardware
crypto, and C6 HE/TWT acceptance. BLE still
needs the chip-specific controller transport, extended advertising, adaptive
hopping, supervision timeout, privacy list behavior, and sleep/wake acceptance.
The genuine OpenThread radio build now passes raw TX/RX, source matching,
energy scan, and sleep/wake through the native C6 port. A secured Thread CLI
exchange and Zigbee-facing firmware qualification still remain. Disputed
ESP32-S3 RF pages remain intentionally unmapped until a revision-
specific primary-source or authorized-hardware probe resolves them. Coexistence
still needs denial/preemption and reset/power-transition stress. Genuine
Wi-Fi/BLE simultaneous traffic is covered on both chips, and the C6 image also
qualifies active three-radio Wi-Fi/BLE/IEEE 802.15.4 ownership.

Do not close issue #338 or describe these radios as vendor-compatible while any
of those gates remains open.
