# ESP32-C6 and ESP32-S3 radio qualification

This directory is the auditable entry point for issue #338. Radio behavior is
host-isolated and deterministic by default; no qualification case opens a host
socket or joins a physical network.

## Reproducible local gates

```sh
python3 scripts/check-radio-sources.py
python3 scripts/check-c6-rf-register-reference.py
python3 scripts/check-c6-custom-driver-contract.py
scripts/fetch-esp-rom-elfs.sh
scripts/capture-c6-rf-oracle.sh
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

## C6 RF register and error reference

[`docs/esp32c6-rf-register-reference.md`](../../docs/esp32c6-rf-register-reference.md)
is the readable index for the union of every ESP32-C6 RF register with explicit
Renvo semantics and every non-UART address in the pinned public-API oracle bus
trace. Its companion `c6-rf-register-reference.json` retains every observed
read value, write value, changed post-value, access count, time bound and
observational PC, as well as modeled masks and reset values where known.
Unknown private fields remain explicitly unknown. This is an evidence-bounded
emulator reference, not a silicon programming manual.

`c6-rf-error-corpus.json` pins the seed, iteration count, minimal distinguishing
mutation and expected rule for all seven causal Wi-Fi RF legality errors. The
structure-aware test named in that corpus runs 2,048 deterministic mutations
and asserts each individual result. Regenerate the register artifacts after a
new pinned oracle capture with:

```sh
scripts/generate-c6-rf-register-reference.py \
  --bus .remu/qualification/c6-rf-oracle/oracle-bus.json \
  --requirements qualification/radio/c6-rf-oracle-requirements.json \
  --json qualification/radio/c6-rf-register-reference.json \
  --markdown docs/esp32c6-rf-register-reference.md
scripts/check-c6-rf-register-reference.py
```

Project-owned native drivers additionally follow
[`docs/custom-radio-driver-contract.md`](../../docs/custom-radio-driver-contract.md).
The C6 gate compares every low-level MMIO call with the driver manifest and RF
reference, rejects trace-only unknown dependencies and access broadening, and
requires named HAL error results.

For issue #356 RF/PHY recovery, the same bounded/streaming bus evidence now
correlates RISC-V accesses with the causative PC without exposing PC to device
models. Autonomous DMA/shared-memory activity deliberately has no PC. Direct
memory writes record safe pre/post values. The C6 RF/PHY models emit the same
evidence from a side-effect-free internal snapshot, so tracing never performs
an extra read of a potentially read-sensitive register. The C6 calibration filter spans
the PHY/MAC, baseband, frontend, MODEM_SYSCON, private PHY, MODEM_LPCON,
analog-I2C, and PHY command-SRAM regions listed in issue #356.
The same genuine-ROM gate streams native C6 radio interrupt-source transitions,
requires assertion and deassertion evidence without stale PC attribution, and
compares that stream byte-for-byte on the repeat-from-reset run.

Issue #353 extends that gate across every interrupt source with an observable
native cause. The machine-readable
[`interrupt-contract.json`](interrupt-contract.json) classifies all thirteen C6
and twelve S3 radio inputs as qualified, modeled-but-unqualified, or unresolved;
the human-readable register, W1C, routing, reset, observed-count, and custom
driver boundary is in
[`docs/esp-radio-interrupts.md`](../../docs/esp-radio-interrupts.md). Wi-Fi, BLE,
BLE modem-sleep, C6 IEEE 802.15.4, and both vendor-independent custom stacks now
stream their exact admitted sources, require balanced alternating
assertion/clear pairs, and compare the whole stream byte-for-byte after reset.
The custom stacks must also match their ordered qualified-source allowlists.
Unknown NMI, coexistence, security, timer, and analog-I2C inputs remain
explicitly unmapped rather than borrowing an ordinary source's behavior.

`capture-c6-rf-oracle.sh` is the separate issue #356 perturbation workflow. It
builds a project-owned, public-API ESP-IDF oracle against the pinned 6.0.2
container and runs it through the genuine C6 rev0 ROM. Flushed UART boundaries
partition cold start, channels 1/6/11, 8/14/20 dBm requested maxima, warm
stop/start, and deinitialize/reinitialize without hooks. Each stage emits a
uniquely tagged raw probe request. `analyze-c6-rf-oracle.py` reduces the ordered
trace to candidate contracts with immutable source provenance and confidence:
the RFPLL frequency strobe/code on `0x600a00c0`, the 43-entry transmit-gain
tuple ending at `0x600a08d4`, and the frontend force-off/release field on
`0x600a0910`. PC values are retained only as observational evidence. The
capture repeats from reset and requires byte-identical result, RF replay, bus,
interrupt, and reduced-analysis artifacts while bounding the complete trace to
64 MiB. This vendor-backed oracle is evidence for the independent HAL; it is
not itself the issue #356 acceptance firmware.

The genuine vendor Wi-Fi qualification workloads exercise a separate observed
59-tuple power/calibration program at the same three gain registers. Renvo
accepts that sequence only when all 59 tuples match in order, and preserves the
last operational `0x4284` HT20 state across the vendor PHY's internal `0x5284`
calibration strobes. These vendor observations do not broaden the custom
driver's exact 43-tuple contract.

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
Its pinned replay contains four distinct modem reset service boundaries. Wi-Fi
RF state is invalidated synchronously at the causal MODEM_SYSCON edge, so the
former delayed invalidation is not counted as a separate coexistence reset
boundary.
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
Valid-FCS secured receive frames are DMA-delivered unchanged even when their
auxiliary security header is truncated. Normal MAC fields may still cause a
hardware automatic ACK, and both the DMA bytes and emitted ACK repeat exactly
from reset. Receive security parsing, decryption and authentication remain in
the guest OpenThread/Zigbee stack; malformed over-air bytes are an ordinary RF
input and never an `illegal radio state` hard error.
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

The same real-ROM OpenThread image completes sixteen consecutive public
raw-link sleep/receive cycles. Its MMIO evidence must contain at least sixteen
native STOP commands and the matching receiver re-arms, and the whole run must
replay exactly. Focused chip tests additionally gate both 802.15.4 clocks after
STOP, prove that RF arriving while asleep cannot touch RX DMA, wake and receive
successfully, and hard-fail if firmware gates the domain while RX is armed.

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
pins the mandatory request, two formation-data frames, and complete beacon
semantics. Clean component builds may additionally emit one observed 47-byte
NWK command (`0x1209`) between the formation frames; that exact control/length
shape is allowed while all other additions are rejected. The gate validates
every hardware-generated FCS, native command and W1C interrupt cause,
coexistence grant, ROM/application coverage and byte-identical
execution-result/RF replay.
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
  cause, and performs a firmware-owned successful retry. Hardware automatic
  ACK airtime uses the same native coexistence grant and legality lifecycle as
  firmware-requested TX; gating its clock mid-ACK is a hard state error.
- A shared BLE H4 controller for C6 and S3 with periodic advertising/scanning,
  initiating, ACL, encryption, data-length, PHY, privacy-facing queries,
  deterministic peers, and controller events.
- A shared Wi-Fi functional engine for C6 and S3 with deterministic peer scans,
  station association, channel/address filters, raw TX/RX, monitor mode,
  power-save state, interrupts, and reset.
- Native Rust and browser/WASI frame-injection and replay APIs. The radio-aware
  browser/WASI call requires and verifies the matching pinned real mask-ROM ELF
  before execution. External host networking remains deliberately absent.

## Landing evidence and post-landing work

The presence of a functional host API is not treated as vendor-firmware
compatibility. The landing gates require pinned, unmodified ESP-IDF
applications to initialize through their real controller/ROM/shared-memory
ABI, and the CI workflows below enforce that boundary. Native C6 and S3 Wi-Fi
now delay TX completion through airtime,
accept matching 802.11 ACK, CTS, and compressed block-ACK control frames, publish vendor status 0 on success
or status 5 at the firmware-programmed timeout, publish status 4 when shared-RF
arbitration rejects the attempt, and leave retry resubmission to the genuine
guest LMAC. Addressed inbound RTS receives a hardware CTS; firmware-programmed
RX BA sessions score QoS sequence numbers with 12-bit wraparound and answer a
compressed BAR from their native 64-bit bitmap. Impossible active BA encodings
are hard legality errors. The C6 and S3 native Wi-Fi crypto tables now retain
all 32 forty-byte key slots, expose firmware-written match/control/key state,
zeroize on reset, and hard-reject a valid slot whose control class is outside
the three encodings emitted by both pinned vendor HALs. A bounded Starlark
workflow now drives both genuine firmware experiments, requires their exact
encrypted ESP-NOW slot-24 peer/interface/key-ID/cipher tuple, and proves that
native CCMP replaces the plaintext payload and firmware MIC reservation before
RF submission. Crypto selection is driven by the durable Protected Frame bit
plus the CCMP ExtIV/key ID; C6 descriptor upper bits remain guest-LMAC-private,
while S3 descriptor bit 29 selects a distinct eight-byte A-MPDU DMA record.
Missing, ambiguous, malformed, or unsupported selection
is a `crypto-key-selection` hard legality error. The workflow pages evidence
with explicit loss accounting and emits only a compact decision artifact; it
does not hook symbols or implement peripherals in Starlark. The C6 PHY page now
latches the native one-microsecond TSF, runs all four
target comparators, derives masked/raw/W1C power events, routes them through the
native Wi-Fi interrupt, and hard-rejects timer-enable/wakeup orders the pinned
HAL never emits. A second bounded Starlark workflow decodes the same sequence.
Descriptor-driven RTS protection now follows the recovered chip-specific queue
bits: hardware emits RTS, waits for CTS, emits the deferred MPDU, then waits for
its ACK or matching-TID block ACK. A missing CTS publishes recovered status 2;
impossible protection of a group, no-response, or control MPDU is a hard state
error. Fragment construction and retry remain in guest LMAC, as demonstrated by
the pinned `ppTxFragmentProc`; the shared native MAC independently qualifies the
ACK window for every fragment. C6 and S3 descriptor word two now links a bounded
two-to-64-MPDU chain. Every node remains hardware-owned, intermediate nodes have
EOF clear, and only the terminal node has EOF set. The machine emits one RF A-MPDU with explicit
MPDU boundaries, accepts a matching compressed BA, and writes the recovered BA
origin, bitmap, and successful-MPDU count; partial BA deliberately leaves
retry/resort/resubmit to guest LMAC. Incoming aggregates enforce one receiver,
transmitter, TID and ACK policy with unique 12-bit sequences, then advance
one native RX descriptor and scoreboard entry per MPDU. Replay, CLI, WASI, the
isolated Starlark peer, and the live agent REPL preserve those boundaries and
reject mixed scalar/aggregate input. The genuine C6 public-firmware gate now
negotiates ADDBA, emits repeated two-MPDU descriptor-linked aggregates with
exact 318-byte boundaries, accepts matching compressed BAs, and completes all
64 bounded UDP packets. The S3 gate negotiates ADDBA and emits 64 genuine
single-MPDU hardware A-MPDU records during BA-session warm-up. The machine
validates and strips each private eight-byte record, emits an explicit-boundary
RF A-MPDU, accepts the matching compressed BA, and completes all 64 packets. A
bounded live descriptor probe records the exact bit-29 record, internal length,
and first MAC words without hooks; licensed `ppAMPDU2Normal` disassembly
independently establishes the eight-byte conversion. Linked two-to-64-MPDU
chains remain covered by direct native regressions. The C6 genuine SoftAP gate now
explicitly enables protocol mask `0x47`, emits HE-capability Extension ID 35
in its native beacon and association response, and accepts an HE20 station;
the paired S3 gate proves its `0x07` non-HE boundary.
The genuine OpenThread radio build now passes raw TX/RX, native CCM* transmit
security, source matching, energy scan, and sleep/wake through the native C6
port. Genuine Zigbee firmware now passes network formation and a peer beacon
exchange over the same native C6 boundary. The full FTD build now reaches
leader role and sends a protected CLI ping; the deterministic virtual-peer
echo response remains. Disputed
ESP32-S3 RF pages remain intentionally unmapped until a revision-
specific primary-source or authorized-hardware probe resolves them. Coexistence
denial, priority preemption, and reset cancellation now have deterministic
chip-level negative-path coverage. Genuine S3 firmware also proves that gating
the register-interface clock after a MAC queue kick preserves autonomous RF
airtime. Genuine Wi-Fi/BLE simultaneous traffic
is covered on both chips, and the C6 image also qualifies active three-radio
Wi-Fi/BLE/IEEE 802.15.4 ownership.

The software-only power-transition torture gate is defined by
`power-transition-contract.json` and can be run end-to-end with
`scripts/qualify-esp-radio-power-transitions.sh`. It composes seven bounded,
pinned genuine-firmware workflows: Wi-Fi and BLE on C6/S3, coexistence on both
chips, and C6 OpenThread. The resulting summary hashes every constituent
qualification artifact. Physical RF, hardware fixtures, host networking, and
symbol dispatch are forbidden by the contract.

The focused layer repeats each C6 and S3 low-power sequencer 256 times and each
active-airtime reset/gate boundary 128 times, then compares complete replay
artifacts from two independent runs. It requires every accepted C6 test frame
to have one cancellation record; on S3 it distinguishes shared-reset
cancellation from the firmware-observed interface-clock behavior that preserves
autonomous airtime. Reset clears armed C6 BLE compares and S3 deferred wake
edges, so neither may produce a stale post-reset interrupt. Duplicate,
overlapping, past-compare, and out-of-order transition requests remain hard
legality failures.

These distinctions are intentional: a clock controlling the firmware register
interface is not automatically the RF power domain. C6 MODEM power loss cancels
airtime; S3 firmware can gate its interface after handing work to the
autonomous MAC. IEEE 802.15.4 must STOP before its clock gate and explicitly
rearm receive after wake. The genuine Wi-Fi workflow supplies chip-level
calibration evidence, while BLE and OpenThread supply the interrupt, traffic,
and wake-reason evidence for those focused semantics.

The following are explicit non-blocking future-work items rather than missing
radio peripherals:

- Additional chip-private BLE host/controller transport variants, adaptive
  channel-map updates, and resolving/accept-list privacy workflows beyond the
  genuine advertising, scanning, connected, encrypted, PHY-update, and
  supervision paths already qualified.
- Additional interrupt/NMI cross-products only when pinned firmware supplies a
  distinct raw/mask/acknowledgement contract; the current qualified and explicit
  unresolved boundary is merge-gated by `interrupt-contract.json`.
- Revision-specific S3 private RF pages once a matching primary source or
  authorized hardware trace resolves them.
- Larger multi-node scenarios, fuzz/metamorphic legality tests, longer replay
  corpora, and performance dashboards.

These extensions retain the real-ROM, LLE, no-hook, deterministic-replay, and
hard-legality requirements. They must not weaken the completed #338 landing
gates or invent behavior for undocumented state.
