# ADR 0002: Direct ELF and firmware-image boot modes

Status: accepted

Every target first supports direct ELF loading for compiler tests. A target
profile defines loadable address ranges, entry state, stack policy, and
secondary-core state.

Firmware-image boot is added per target when selected vendor SDK cases require
it. Public boot ROM implementations may execute directly. Otherwise, narrow
versioned ROM-call facades are allowed and must be listed in machine fidelity
metadata.
