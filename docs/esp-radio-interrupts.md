# ESP radio interrupt contract

This document is the human-readable companion to
[`qualification/radio/interrupt-contract.json`](../qualification/radio/interrupt-contract.json).
That file is the merge-gated source of truth. It binds every public radio
interrupt-source name to one of three states:

- `qualified`: a native cause, mask/enable, acknowledgement, reset behavior,
  and genuine pinned-firmware assertion/deassertion replay are known;
- `modeled-unqualified`: a bounded model exists, but the pinned qualification
  firmware does not assert the source, so custom drivers must not depend on it;
- `unresolved`: the public source name is inventoried, but Renvo does not infer
  private cause semantics from the name alone.

All workflows execute the genuine pinned mask ROM and vendor firmware. The
interrupt stream is observational only: it cannot change device state, inject
an interrupt, or perform an extra MMIO read.

## Qualified source matrix

| Chip | Source | Native name | Cause and enable boundary | Acknowledgement | Genuine workflow |
|---|---:|---|---|---|---|
| C6 | 0 | Wi-Fi MAC | MAC event `+0x0c48` gated by mask `+0x0c40` | W1C `+0x0c4c` | Wi-Fi association/TWT |
| C6 | 2 | Wi-Fi power | PHY raw `+0x0ac` gated by enable `+0x0a8`, reflected at status `+0x0b0` | W1C `+0x0b4` | Wi-Fi association/TWT |
| C6 | 4 | BT MAC | enabled BLE baseband cause or pending controller HCI output | baseband W1C or HCI consumption | BLE and BLE sleep |
| C6 | 5 | BT baseband | raw0/1 `+0x30c/+0x31c` gated by enable0/1 `+0x304/+0x314` | W1C `+0x308/+0x318` | BLE and BLE sleep |
| C6 | 7 | LP timer | BLE-modem raw/status `+0x024/+0x034`, enable `+0x010` bit 18 | W1C `+0x014` bit 18 | BLE sleep |
| C6 | 12 | Zigbee MAC | IEEE 802.15.4 event `+0x064` gated by mask `+0x060`, bits 0–12 | W1C `+0x064` | native IEEE 802.15.4 |
| S3 | 0 | Wi-Fi MAC | MAC event `+0x0c3c` gated by mask `+0x0c34` | W1C `+0x0c40` | Wi-Fi association |
| S3 | 8 | RWBLE | RWBLE cause `+0x010`; the ROM dispatches enabled causes configured at `+0x00c` | W1C `+0x018` or the recovered ECO diagnostic acknowledgement | BLE and BLE sleep |

Offsets are relative to the region named in the machine-readable contract.
Source 4 on S3 is intentionally absent from the qualified table: Renvo models
the controller HCI-output level, but the pinned BLE workflows do not assert the
matrix source.

## Observed replay values

The issue-353 baseline used the exact successful ROM/vendor images from the
issue-356 landing plus fresh S3 builds. The first run observed these balanced
assertion/deassertion pairs:

| Workflow | C6 pairs | S3 pairs |
|---|---|---|
| Wi-Fi | source 0: 53; source 2: 37 | source 0: 10 |
| BLE active | source 4: 38; source 5: 38 | source 8: 100 |
| BLE modem sleep | source 4: 40; source 5: 40; source 7: 37 | source 8: 74 |
| IEEE 802.15.4 | source 12: 12 | not present |
| Custom stack | source 0: 2; source 4: 2; source 5: 2; source 12: 12 | source 0: 2; source 8: 2 |

Counts may change when a pinned firmware workload changes. They are recorded
as technical observations, not guessed hardware timing. The durable contract
requires:

1. the exact admitted source set for that workflow;
2. the exact stable signal name for every source;
3. alternating levels with no duplicate assertion or duplicate clear;
4. at least one assertion/clear pair for every admitted source;
5. equal assertion and deassertion counts, ending deasserted; and
6. a byte-identical interrupt stream on repeat execution from reset.

`scripts/check-radio-interrupt-trace.py` enforces these rules. An unexpected
extra source is a hard qualification failure, not an automatic expansion of
the supported model.

## Routing, NMI, and priority boundary

On C6, qualified peripheral levels enter the native interrupt matrix at
`0x6001_0000` and then the PLIC/CPU interrupt path. The existing matrix and
PLIC models preserve routing, pending status, enable, priority, and threshold
behavior. None of the pinned radio workflows establishes a distinct radio NMI
cause, so source 1 or 6 is not aliased to an ordinary MAC/baseband level.

On S3, qualified levels enter both core views of the interrupt matrix at
`0x600c_2000`. Normal route, status, reroute, core selection, and reset
behavior is modeled. CPU interrupt line 14 participates in the World
Controller NMI mask, but no pinned radio workflow currently selects that
cross-product. Consequently this document does not claim a qualified
radio-specific NMI route.

This distinction is deliberate: a source called `*-nmi` in the public header
proves that an input exists, not which private raw bit asserts it or how that
bit is acknowledged.

## Reset and stale-delivery rules

Each qualified peripheral reset clears its raw causes, masks/enables where the
native block resets them, pending DMA/scheduler work, and the machine-facing
level. A repeat run starts with a new machine and must reproduce the entire
stream byte-for-byte. A focused S3 RWBLE regression also proves both paths:

- W1C acknowledgement emits exactly one deassertion; and
- a device reset while asserted emits a deassertion on the next scheduler
  service, with no stale post-reset delivery.

The generic interrupt matrices reset their route and status state independently
of the radio devices. The machine recomputes each qualified level from native
device state rather than retaining a host-owned completion flag.

## Unresolved sources

The following inputs remain unmapped until genuine firmware or an authorized,
reproducible trace supplies a cause/mask/clear contract:

- C6: Wi-Fi MAC NMI (1), Wi-Fi baseband (3), BT baseband NMI (6), coexistence
  (8), BLE timer (9), BLE security (10), and I2C master (11).
- S3: Wi-Fi MAC NMI (1), Wi-Fi power (2), Wi-Fi baseband (3), BT baseband (5),
  BT baseband NMI (6), RWBT (7), RWBT NMI (9), RWBLE NMI (10), and I2C master
  (11). BT MAC (4) is modeled but not yet qualified.

Coexistence ownership, BLE security completion, and analog-I2C calibration are
already modeled through other native boundaries. That does not justify
synthesizing their similarly named interrupt inputs.

## Custom-driver controls

A project-owned driver may depend only on a `qualified` entry. It must:

- enable the documented peripheral cause before relying on delivery;
- route the exact native source through the chip interrupt fabric;
- read the native raw/status register to identify the cause;
- acknowledge only a cause it observed, using the documented W1C path;
- tolerate level reassertion when new work arrives after acknowledgement;
- disable/cancel work before resetting or gating its radio domain; and
- treat any unqualified source, undocumented route, or post-reset completion as
  unsupported rather than adding a host-side shortcut.

Adding a source requires updating the machine-readable contract, focused
raw/mask/route/W1C/reset tests, a genuine-firmware workflow, deterministic
interrupt replay, provenance, and this document in the same change.

The vendor-independent custom-stack gate additionally binds each driver image
to an ordered `qualified_interrupt_sources_used` allowlist. Its runtime stream
must contain exactly that set and replay byte-for-byte after reset; merely
declaring a source in the firmware requirements cannot admit it.
