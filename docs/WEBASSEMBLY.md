# WebAssembly and browser API

Renvo exposes its deterministic machine runners as a WASI Preview 2 reactor
component. The component is built from `crates/remu-web`, and its canonical API
is the versioned WIT world in `crates/remu-web/wit/renvo.wit`.

This architecture keeps the emulator independent of browser globals:

```text
firmware bytes + bounded options
              │
              ▼
JavaScript API (`web/src/remu.js`)
              │
              ▼
jco-generated ES module + WASI browser shim
              │
              ▼
Renvo WASI Preview 2 component
              │
              ▼
existing CPU, bus, device, and machine crates
```

Browsers execute core WebAssembly modules but do not yet instantiate Component
Model binaries directly. The web build therefore uses Bytecode Alliance `jco`
to transpile the Rust-produced component into an ES module and core Wasm files.
The generated module uses `@bytecodealliance/preview2-shim` for its WASI imports.

## Build and run

Install a stable Rust toolchain, Node.js 20.19 or newer, and npm. Then run:

```sh
rustup target add wasm32-wasip2
cd web
npm ci
npm run build
npm run dev
```

`npm run build` performs three steps:

1. builds `remu-web` as a release WASI Preview 2 component;
2. transpiles the component with `jco` using browser-compatible output; and
3. bundles the static example with Vite.

The production site is written to `web/dist`. Generated component bindings are
written to `web/generated` and intentionally remain untracked.

## JavaScript API

The hand-written wrapper accepts `Uint8Array`, `ArrayBuffer`, or any typed-array
view. Every execution is bounded by `maxInstructions`; the default is one
million architectural actions.

```js
import { Renvo } from "./src/remu.js";

const targets = Renvo.listTargets();
const firmware = new Uint8Array(await file.arrayBuffer());
const bootRom = new Uint8Array(await romFile.arrayBuffer());

const result = Renvo.runElf("esp32c6", firmware, {
  maxInstructions: 1_000_000,
  deadlineTicks: 2_000_000,
  stimuli: [
    { at: 100, pin: 9, value: "zero" },
    { at: 200, pin: 9, value: "one" },
  ],
});

const radioResult = Renvo.runRadioElf("esp32c6", firmware, bootRom, {
  maxInstructions: 1_000_000,
  deadlineTicks: 2_000_000,
  radioFrames: [{
    at: 500,
    protocol: "ieee802154",
    centerKHz: 2_405_000,
    bandwidthKHz: 2_000,
    phy: "ieee802154-oqpsk-250k",
    bytes: new Uint8Array([/* complete PSDU including FCS */]),
    powerDbm: 0,
  }],
});

console.log(radioResult.radio.events);
```

The public methods are:

- `Renvo.listTargets()` — returns the target manifest array.
- `Renvo.inspectElf(bytes)` — parses a supported 32-bit little-endian ELF.
- `Renvo.runElf(target, bytes, options)` — runs ELF firmware on the matching
  machine model and returns the normal structured run result.
- `Renvo.runRadioElf(target, bytes, bootRom, options)` — runs ESP32-C6 or
  ESP32-S3 firmware with timestamped, host-isolated RF input and returns both
  the normal result and versioned packet/coexistence replay evidence. `bootRom`
  must be the matching pinned revision-zero mask-ROM ELF; its SHA-256 is
  verified before parsing or execution.
- `Renvo.runIntelHex(target, bytes, options)` — runs Intel HEX on PIC16F15376 or
  EFM8BB52F32G.
- `Renvo.*Json(...)` variants — return the exact JSON boundary text without
  parsing it in JavaScript.

WIT `u64` inputs are converted to JavaScript `BigInt` by the wrapper. Parsed
JSON results follow the existing CLI artifact schema; callers that require
integers above JavaScript's safe integer range should retain the `*Json` text.

## Browser execution contract

- Firmware never needs host filesystem access; bytes cross the component
  boundary directly.
- Runs are deterministic and bounded, but they are synchronous CPU work. Use a
  Web Worker for large instruction budgets. The included example does this.
- VCD streaming, GDB sockets, Docker corpus compilation, and host filesystem
  persistence remain native-host features. Browser results still include CPU
  state, UART/USB output, stop reason, statistics, and the trace digest.
- WASI Preview 2 browser shims are still experimental. The WIT interface is the
  stable project-owned contract; generated JavaScript should always be rebuilt
  from the pinned npm dependencies.
