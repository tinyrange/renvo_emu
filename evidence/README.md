# Expansion evidence

`targets.toml` is the executable evidence index for the seven targets in the
frozen expansion plan. It pins the exact part identity, architectural manuals,
compiler or device-pack version, initial sample source, memory sizes, and the
peripheral slice that acceptance is allowed to require.

Each target entry also records its exact load format, address map, reset
assumptions, vectors, selected pins, interrupt routes, stable VCD hierarchy,
vendor-sample licence, fidelity tier, and unsupported behavior. The
comprehensive validator consumes these fields directly rather than relying on
the prose plan.

URLs are upstream vendor or project sources. Archive hashes are recorded when
the referenced input is redistributed into a container build; Git sources use
full commit IDs. Updating a pin requires reviewing the corresponding emulator
and qualification assumptions.
