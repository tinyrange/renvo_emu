# Third-party inputs

Renvo Emulator's tracked project code is licensed under `MIT OR Apache-2.0` unless a
file says otherwise. The repository does not redistribute compiler images,
official firmware binaries, or restricted vendor SDK source.

Qualification scripts may fetch checksummed third-party inputs into the
ignored `.remu/` directory. Those inputs are separate works governed by their
own licenses and are not relicensed by Renvo Emulator. The relevant source URL,
revision, checksum, and license treatment are recorded in `evidence/targets.toml`,
`firmware/`, the target qualification README, or `toolchains/README.md`.

Notable input categories include:

- official MicroPython firmware under the MicroPython project license;
- EEMBC CoreMark and MQuickJS sources fetched at pinned revisions;
- vendor and community examples fetched on demand where their licenses permit;
- GNU, LLVM, Rust, SDCC, Espressif, TI, and Microchip compiler/device-pack
  inputs used to construct local Docker images.

Some vendor tools and samples impose device-use or redistribution restrictions.
The corresponding scripts build or stage them locally only; users are
responsible for reviewing and accepting the applicable terms. In particular,
the XC8 Docker image is a local-only build input and must not be published.
