//! Resolve the version reported by `mdlr --version`.
//!
//! At release time GoReleaser sets `MDLR_VERSION` to the git tag it is building
//! (see `.goreleaser.yaml`), which is the single source of truth for a release.
//! Locally — `cargo build`, `task link`, editors — the env var is unset and we
//! fall back to the workspace `version` in `Cargo.toml` (`CARGO_PKG_VERSION`).
//!
//! The value is re-exported as the `MDLR_VERSION` compile-time env so `cli.rs`
//! can hand it to clap's `#[command(version = ...)]`.

fn main() {
    // Re-run this script (and thus rebuild the version string) whenever the
    // release tag changes, so a cached build doesn't bake in a stale version.
    println!("cargo:rerun-if-env-changed=MDLR_VERSION");

    let version = std::env::var("MDLR_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    println!("cargo:rustc-env=MDLR_VERSION={version}");
}
