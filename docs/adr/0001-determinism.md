# ADR 0001: Deterministic simulation

Status: accepted

Renvo Emulator uses a single logical event timeline. Events are ordered by
`(simulation_time, insertion_sequence)`. Host wall time, host threads, hash-map
iteration order, and filesystem ordering must not affect simulated behaviour.

Secondary CPU cores initially remain parked. When multicore execution is
enabled, cores receive deterministic round-robin instruction quanta on the same
logical timeline.
