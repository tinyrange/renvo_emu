# Releases and runtime images

The release workflow is intentionally separate from the compiler toolchains.
For a `v*` tag it builds the `remu` CLI with the locked workspace dependency
set for Linux/amd64 and Linux/arm64, publishes SHA-256 checksums, and creates a
quick-start archive containing the source, a Docker-built CH32V003 ELF, and
deterministic JSON/VCD run artifacts. The runtime image is built from
[`runtime/Dockerfile`](../runtime/Dockerfile); it contains only the CLI and a
minimal Ubuntu runtime, uses UID/GID `10001`, and has no compiler binaries.
The release build injects the tag version (without its leading `v`) into the
CLI and checks that the amd64 binary reports the same version.

The image is published as:

```text
ghcr.io/tinyrange/renvo_emu:<tag>
ghcr.io/tinyrange/renvo_emu:latest
```

Both tags are multi-architecture manifests. Architecture-specific image tags
and source/revision labels remain available for provenance checks. The release
asset checksum file covers both architecture-specific binaries and the
quick-start archive. A quick-start run is documented in
[`examples/quickstart`](../examples/quickstart/README.md); extracting the
archive provides a ready-to-run `quickstart/build/quickstart.elf` plus its
build provenance, result, waveform, and per-file checksums.

The image should be treated as an execution environment, not a build
environment. Firmware compilation continues to use the immutable, pinned
Docker images referenced by `toolchains/*.toml`.
