# Custom radio driver contract

This contract governs project-owned firmware that drives an emulated radio
through native MMIO rather than a vendor runtime. Its purpose is to make every
hardware dependency, recovered fact, emulator convention, state transition,
and failure path reviewable before a driver becomes an acceptance artifact.

It does not turn emulator behavior into a silicon specification. Custom radio
drivers in this repository are software-emulator qualification tools. They are
not flashing utilities, physical RF tests, or evidence of output power,
spectral compliance, calibration, coexistence fidelity, or regulatory safety.

## Required artifacts

Every custom radio driver must include:

1. A single address header containing every peripheral address literal used by
   the driver. Ordinary values, bit masks, table data, and protocol constants
   remain beside their implementation.
2. A machine-readable MMIO manifest with one entry per exact register or
   bounded register region.
3. A static checker that compares the address header, manifest, source calls,
   and the chip register reference.
4. Stable named result codes for all driver-visible failures. Anonymous
   negative returns are forbidden inside the HAL.
5. A deterministic exact-image qualification that exercises initialization,
   reset, normal TX/RX, and every claimed protocol or crypto path.
6. Human documentation of ordering, ownership, value constraints, side
   effects, timeouts, and expected errors.

The ESP32-C6 implementation is
[`qualification/radio/c6-rf-probe/mmio-contract.json`](../qualification/radio/c6-rf-probe/mmio-contract.json).

## Evidence tiers

| Tier | Permitted use | Required evidence |
|---|---|---|
| `trace-causal` | Native radio register used by a recovered sequence | The address is trace-observed, has modeled semantics, and the driver use agrees with the pinned causal trace. |
| `emulator-contract` | Native radio register needed by a project-owned path that is not present in the initialization oracle | The address has explicit functional semantics and deterministic regression/acceptance coverage. The manifest must say that it was not observed in the oracle. |
| `platform-contract` | UART, interrupt controller, DMA fabric, or another non-radio platform dependency | A public platform contract or explicit Renvo peripheral model supports the use. |

An address that is merely present in a trace but still marked `observed-only`
is not a driver contract. It may be researched, documented, and replayed, but
custom firmware cannot depend on it until its semantics and failure behavior
are modeled explicitly.

## Manifest entry requirements

Each dependency declares:

- symbolic name and absolute base address;
- bounded byte span and stride;
- the narrowest actual access direction: `read`, `write`, or `read-write`;
- radio or platform domain;
- evidence tier and provenance;
- lifecycle phases in which access is legal;
- technical function and accepted value constraints;
- hardware/model side effects; and
- deterministic failure behavior.

The manifest is deliberately more specific than a datasheet-style address
list. A register name without ordering, ownership, and error semantics is not
enough to safely build an acceptance driver.

## Source controls

The C6 checker enforces these rules:

- peripheral address literals are legal only in `hal/c6/registers.h`;
- every address symbol appears exactly once in both the header and manifest;
- every `c6_read32` or `c6_write32` target contains exactly one declared
  register symbol;
- source access cannot exceed the declared direction;
- the manifest cannot declare broader access than the source actually uses;
- declarations cannot be unused or overlap;
- dynamic tables require an aligned, bounded span and stride;
- every radio word resolves to a semantically implemented reference entry;
- `trace-causal` words must also be present in the pinned trace;
- trace-only semantic unknowns are rejected; and
- numeric negative returns in the HAL are rejected in favor of named results.

Run the gate directly with:

```sh
scripts/check-c6-custom-driver-contract.py
```

## Lifecycle and ownership

Custom drivers must separate these phases even when the functional emulator
completes a hardware action synchronously:

| Phase | Required discipline |
|---|---|
| Initialization | Preserve unrelated clock/interrupt state with documented read-modify-write operations. Do not treat reset state as configured RF state. |
| Reset | Stop RX and require the modeled ready acknowledgement. A MAC-only reset retires MAC DMA, queue, interrupt, and crypto ownership but preserves independently programmed RF calibration. A modem-domain Wi-Fi reset also invalidates cached RF state and requires complete RF reconfiguration. |
| RF configuration | Force the frontend off, complete channel programming, write the full ordered gain table, then explicitly release the frontend. |
| Receive | Prepare a valid internal-memory descriptor before transferring RX ownership. A descriptor may remain armed while the frontend is forced off; it is DMA storage, not an airtime request. Actual receive airtime still requires the frontend to be released. |
| Transmit | Validate frame bounds, prepare descriptor contents, transfer one queue ownership, require matching event and queue completion, clear both, and bound the wait. |
| Security | Populate every word of a supported native key slot before setting its valid bit. Selection must be unique for peer, interface, key ID, and cipher. |
| Diagnostic | UART checkpoints may report state but must not create radio state or bypass normal MMIO. |

The emulator recognizes two independently bounded gain-program families. The
project-owned custom driver must emit one of the exact 43-tuple 8, 14, or
20 dBm profiles declared by its manifest. The pinned genuine ESP-IDF Wi-Fi
workload emits a distinct exact 59-tuple sequence: 32 transmit-power tuples
followed by 27 calibration tuples, all with `0x40200000` as the first word.
Only a byte-for-byte complete vendor sequence establishes its observed
21 dBm (`84` quarter-dBm) ceiling. Partial, reordered, or cross-family tuples
do not establish calibration.

`RFPLL_CHANNEL_CONTROL` mode `0x4284` plus the start bit is the observed
operational HT20 channel selection and advances the Wi-Fi RF configuration
generation. Genuine firmware also strobes mode `0x5284` during internal PHY
calibration, then restores `0x4284` without a new start edge. That calibration
mode does not replace or invalidate the last operational Wi-Fi channel state.

Poll loops must have a deterministic bound and a named timeout result. A
missing event is never converted into success. W1C events and queue ownership
must be cleared before reuse so stale state cannot prove a later operation.

## Error policy

There are three distinct error layers:

1. Driver validation rejects bad API input before MMIO, such as an unsupported
   channel, power, frame length, or output buffer.
2. Driver operation results report recoverable bounded outcomes, such as no RX
   packet or a TX response timeout.
3. Emulator legality errors terminate impossible hardware state, such as a
   disabled clock domain, incomplete/stale calibration, invalid DMA ownership,
   ambiguous crypto selection, or an uncleared queue kick.

Drivers must not catch a legality error and reinterpret it as an ordinary RF
outcome. Conversely, ordinary medium behavior such as no peer response, CCA
busy, or a missing packet is not evidence of an illegal register sequence.

## Bounded regions

A dynamic register region must declare its complete geometry. The current C6
driver uses one such region: slot zero of the native Wi-Fi crypto table is ten
32-bit words (`span_bytes: 40`, `stride_bytes: 4`). Code may derive addresses
only from the declared base. Expanding to another slot or additional word
requires a manifest change, documentation, checker review, and qualification.

This rule prevents a convenient base pointer from silently becoming authority
to access an entire undocumented peripheral page.

## Adding or changing a dependency

Use this sequence for every register change:

1. Establish evidence and add or improve the chip register reference.
2. Decide the evidence tier. Never promote `observed-only` state by naming it
   in driver code first.
3. Add the exact address to the central address header.
4. Add a manifest entry with the narrowest access and lifecycle scope.
5. Add named negative-path coverage before using the register in an acceptance
   path.
6. Run the static contract checker, source/provenance checker, relevant unit
   tests, and exact-image qualification.
7. Review the generated diff for new address literals, broader ranges,
   weakened timeouts, or physical-hardware language.

Any intentional expansion should be obvious as an address-header line, a
manifest entry, a functional test, and documentation in the same change.

## C6 current boundary

The current independent C6 driver declares 24 MMIO dependencies: 18 radio
registers and six platform/console registers. Seventeen radio dependencies are
`trace-causal`; the 40-byte crypto slot is an explicit `emulator-contract`
backed by project-owned WPA2/CCMP qualification. No trace-only unknown register
is permitted. The address surface includes no physical-radio execution path.
