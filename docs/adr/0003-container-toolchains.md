# ADR 0003: Docker-only corpus toolchains

Status: accepted

Firmware corpus compilation must run in Docker. The host compiler is never an
implicit fallback.

Toolchain specifications use a pinned image reference. Before execution, Renvo
resolves and records the Docker image ID. Build containers:

- run without a network;
- mount source read-only;
- mount a dedicated output directory read-write;
- use a deterministic working directory and explicit environment;
- receive CPU, memory, process, and wall-time limits;
- run with dropped Linux capabilities and `no-new-privileges`.

Each artifact records the image reference, immutable image ID, argv, relevant
environment, target, flags, and SHA-256 hashes of build inputs and outputs.
