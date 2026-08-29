//! Injects the release-provided version into the CLI at compile time.

use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=REMU_RELEASE_VERSION");
    let version = env::var("REMU_RELEASE_VERSION")
        .unwrap_or_else(|_| env::var("CARGO_PKG_VERSION").expect("Cargo supplies package version"));
    println!("cargo:rustc-env=REMU_BUILD_VERSION={version}");
}
