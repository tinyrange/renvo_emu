# SDCC MCS-51 toolchain

This local-build recipe compiles SDCC 4.5.0 from the official SourceForge
source archive. The archive URL is:

`https://downloads.sourceforge.net/project/sdcc/sdcc/4.5.0/sdcc-src-4.5.0.tar.bz2`

The required SHA-256 is
`d5030437fb436bb1d93a8dbdbfb46baaa60613318f4fb3f5871d72815d1eed80`.
The Dockerfile verifies that digest before extraction. The reviewed image ID
is recorded by `toolchains/sdcc-mcs51-efm8bb52.toml`; a fresh checkout may use
the named local fallback, which Renvo Emulator resolves to an immutable ID before use.
Compilation runs with `--network=none`, a read-only root, and deterministic
locale/source-date settings through Renvo Emulator's corpus runner.

Build the image locally when the pinned image is unavailable:

```sh
docker build --pull=false --tag remu/sdcc-mcs51:4.5.0 toolchains/sdcc-mcs51
docker image inspect --format '{{.Id}}' remu/sdcc-mcs51:4.5.0
```

No specification edit is required after a local build. SDCC is GPL-2.0; the
image is a reproducible build input rather than a repository-distributed
vendor binary.
