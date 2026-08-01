# Releases and runtime images

The release workflow is intentionally separate from the compiler toolchains.
For a `v*` tag it builds the `remu` CLI with the locked workspace dependency
set for Linux/amd64 and Linux/arm64, publishes SHA-256 checksums, and creates a
quick-start source archive. The runtime image is built from
[`runtime/Dockerfile`](../runtime/Dockerfile); it contains only the CLI and a
minimal Ubuntu runtime, uses UID/GID `10001`, and has no compiler binaries.

The image is published as:

```text
ghcr.io/tinyrange/renvo_emu:<tag>
ghcr.io/tinyrange/renvo_emu:latest
```

Both tags are multi-architecture manifests. Architecture-specific image tags
and source/revision labels remain available for provenance checks. The release
asset checksum file covers each architecture's binary. A quick-start run is
documented in [`examples/quickstart`](../examples/quickstart/README.md).

The image should be treated as an execution environment, not a build
environment. Firmware compilation continues to use the immutable, pinned
Docker images referenced by `toolchains/*.toml`.
