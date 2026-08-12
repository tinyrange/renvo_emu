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
scripts/qualify-esp-radio-openthread-cli-vendor.sh esp32c6
scripts/qualify-esp-radio-zigbee-vendor.sh esp32c6
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
open-system station association. The same run now streams a region-filtered
calibration log instead of retaining a broad trace in memory. C6 must program
the PHY analog-I²C command SRAM and complete RFPLL charge-pump, frequency,
power-detector, and IQ-estimator paths. S3 must complete PHY TX-DC,
packet-detector, and FE IQ-estimator paths while exercising its AGC, NRX, and
baseband pages. The calibration-region list and log digest are part of the
versioned result. Each chip transmits exact probe,
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
Firmware radio-reset assertions cancel pending native work, active ownership,
and its exact in-flight medium transmission. Firmware clock/power gating also
cancels active ownership and airtime. The append-only artifacts retain the
reset or power-down boundary and truncated-airtime evidence. Focused C6/S3
machine tests force priority preemption and denial: the preempted packet is
truncated by stable transmission ID, while denied work creates no airtime.

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

Current BLE evidence uses two complementary viewpoints on both chips. Each
genuine ESP-IDF/NimBLE probe initializes the real controller and PHY, runs
legacy advertising, then executes BLE 5 extended advertising across a native
primary `ADV_EXT_IND` and firmware-described 2M `AUX_ADV_IND`. The gate checks
the exact per-chip primary channel, AuxPtr channel/offset/PHY, random AdvA, ADI,
and `Renvo-EXT` payload. A scripted central then sends `CONNECT_IND` and drives
the genuine controller through hopped connection events, LL version and feature
exchange, a 251-octet data-length exchange, and an ACL/L2CAP ATT MTU request.
Hardware-owned SN/NESN handling retires the transmitted ACL descriptor and the
controller emits the exact MTU response. On both chips the scripted central
then drives `LL_ENC_REQ` and START_ENC using the probe LTK. Native ECB derives
the session key, data-channel CCM authenticates encrypted empty/control PDUs,
and the public GAP callback must report encryption success before an encrypted
`LL_TERMINATE_IND` reports reason `0x13`; scanning restarts after disconnect. It then scans
through the vendor-owned RX ring; its
public extended GAP callback must return the exact injected advertising data
and -80 dBm received power. Each freestanding
probe independently programs its chip's scheduler, descriptor/shared-memory
layout, scan window, RX descriptor and payload buffer, then validates ownership
transitions, metadata, RX/END interrupts, and W1C clearing. Each run requires
identical radio replay on reset.

The same vendor image also runs a separate silent-peer workflow on both chips.
Its generated `CONNECT_IND` programs a 100 ms supervision timeout, keeps the
link alive through the public connection and MTU callbacks, then removes the
central. Genuine controller firmware must report HCI reason `0x08` through the
public GAP disconnect callback (NimBLE reason 520) and restart scanning. C6
additionally validates the firmware-decoded schedule-result bit that promotes
the native link from establishing to connected. An unexplained duplicate RX
completion, a continuation without an initial completion, or a completion after
firmware releases the schedule is an immediate radio-legality error. A valid
encrypted continuation in the same native event is accepted. The silent-peer
result and RF replay must repeat exactly from reset.

The vendor BLE gate also builds a dedicated modem-sleep variant from the same
public ESP-IDF/NimBLE probe. C6 runs the firmware-selected 100 kHz main-XTAL
sleep timer at `0x600a_e000`: firmware clears source 18, programs a future RTC
compare, enables it, and may either cancel the armed wake or service and clear
the pending interrupt. S3 runs the firmware-selected 1 MHz main-XTAL clock at
`0x6004_2000`: firmware programs the duration, requests RF sleep, observes the
hardware acknowledgement, requests wake, and receives distinct wake-start and
deferred wake-end RWBLE causes. These are firmware-observed legal state
machines, not runtime-learned behavior. A past compare, enable-before-compare,
invalid clear phase, or out-of-order S3 sleep/wake command terminates the guest
with an `illegal radio state` hard error. The genuine C6 run reaches 20 million
instructions with at least 100 native BLE transmissions; S3 reaches 16 million
with at least four. Both require their pinned real ROM, all public advertising
milestones, ROM and application-entry coverage, and byte-identical result and
RF replay from reset.

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
On transmit, the DMA length is the complete over-the-air PSDU length. Firmware
reserves its final two bytes while the peripheral calculates and writes the
FCS; secured frames similarly reserve their selected MIC before the hardware
encrypts the payload and replaces that reservation. The ESP-IDF, OpenThread,
coexistence and custom-stack gates pin their exact FCS-bearing frames. Zigbee
pins the frame sequence, controls, lengths, PAN and complete beacon body,
validates every hardware FCS, and then requires an exact replay of the complete
run because stack-owned sequence and secured NWK bytes are intentionally
randomized between clean firmware builds.

The genuine OpenThread gate additionally drives the public raw-link MAC-key and
frame-counter APIs into the native ESP-IDF radio port. The vendor port writes
the extended-address nonce source, all four AES key words, payload offset 15,
and the security-enable transition before DMA submission. Qualification pins
the resulting ENC-MIC-32 PSDU, proves that the complete MAC and auxiliary
security headers remain clear, and requires byte-identical execution-result
and RF replay artifacts. This closes the OpenThread-to-native-transmit-security
peripheral handoff; it does not claim a full Thread network exchange or move
receive decryption out of the guest stack.

The complementary full-stack OpenThread gate uses the default FTD and platform
key-reference configuration. Native SPI1 user commands erase, program, and
read the NVS partition; the PSA persistent-key import and an immediate NVS
round trip must both succeed. The application then commits a fixed CLI
dataset, enables Thread, becomes leader, and lists its mesh addresses. A
bounded event-driven Starlark peer reacts only to emitted RF frames: it performs
the authenticated Parent/Child exchange and returns a protected unicast ICMPv6
echo reply. The genuine CLI must expose the child and report its echo response.
All guest radio configuration still traverses native CCA/TX/RX, AES, W1C,
hardware-FCS, and coexistence paths. The peer has explicit JSON state and no
machine, memory, register, symbol, filesystem, clock, or network capabilities;
`--radio-repl` optionally opens a debugger in its callback scope. The result
and RF event stream must replay byte-for-byte from reset.

The Apache-2.0 ESP Zigbee 2.0.3 gate boots its coordinator application through
the genuine C6 rev0 ROM, forms a native channel-11 network with PAN ID 63100,
then receives an injected beacon request and emits the coordinator beacon. It
pins the emitted frame shapes and complete beacon semantics, validates every
hardware-generated FCS, native command and W1C interrupt cause, coexistence
grant, ROM/application coverage and byte-identical execution-result/RF replay.
The component lock, license and all
three C6 library archives are independently hashed; the retired ZBOSS
dependency is rejected by the qualification gate.

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
crypto, and C6 HE/TWT acceptance. BLE still needs the remaining chip-specific
controller transport, adaptive hopping, and privacy list behavior.
The genuine OpenThread radio build now passes raw TX/RX, native CCM* transmit
security, source matching, energy scan, and sleep/wake through the native C6
port. Genuine Zigbee firmware now passes network formation and a peer beacon
exchange over the same native C6 boundary. The full FTD build now reaches
leader role and sends a protected CLI ping; the deterministic virtual-peer
echo response remains. Disputed
ESP32-S3 RF pages remain intentionally unmapped until a revision-
specific primary-source or authorized-hardware probe resolves them. Coexistence
denial, priority preemption, reset cancellation, and active clock/power gating
now have deterministic chip-level negative-path coverage; genuine-firmware
power-transition stress still remains. Genuine Wi-Fi/BLE simultaneous traffic
is covered on both chips, and the C6 image also qualifies active three-radio
Wi-Fi/BLE/IEEE 802.15.4 ownership.

Do not close issue #338 or describe these radios as vendor-compatible while any
of those gates remains open.
