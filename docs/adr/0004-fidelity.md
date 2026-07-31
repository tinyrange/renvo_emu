# ADR 0004: Model fidelity

Status: accepted

Machine and peripheral manifests declare one of:

- `architectural`: intended to follow an architecture specification;
- `functional`: useful register and signal behaviour with approximate timing;
- `educational`: intentionally simplified behaviour.

Unsupported registers fault or emit a structured diagnostic according to the
machine policy. They must not silently behave as implemented hardware.
