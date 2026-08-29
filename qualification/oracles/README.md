# Oracle fixtures

Oracle fixtures are small, versioned records of observations made outside the
Renvo Emulator. They make hardware and differential evidence reviewable and
allow CI to replay the comparison without a board attached.

The format is `remu.oracle.v1`. Each fixture records:

- the target and physical board identity;
- the reset and initial-state assumptions;
- the exact firmware source inputs and image hash;
- the stimulus and observation records;
- the equivalence and timing policy; and
- capture-tool and provenance details.

`hardware_capture` fixtures contain measurements from a physical board.
`external_emulator` fixtures will contain results from QEMU, Renode, or a
vendor simulator. `remu_reference` is reserved for a generated reference that
does not claim to be independent evidence.

The first checked fixture is the 40-case edge-corpus run on an M5Stack NanoC6
(ESP32-C6). The original binary capture remains in the local `.remu/` working
directory because it is a build artifact; the fixture carries its SHA-256 and a
canonical copy of every observation. No vendor binary or board dump is stored
in the repository.

CI only replays this checked, non-radio CPU result set. It never connects to a
physical board, transmits RF, or invokes the hardware capture script.

Validate every checked fixture from the repository root with:

```sh
python3 scripts/validate-oracles.py
```

The validator checks the record shape, source-input digest, capture digest, and
every observation against `corpus/edge_cases/manifest.tsv`. A changed GPIO,
timer, UART, or exception result must therefore be reported as structured
evidence instead of silently becoming a new expectation.

This is the first slice of issue #26. A second physical board, independent
emulator lanes, and dedicated reset/GPIO/timer/UART/exception fixtures remain
follow-up work.
