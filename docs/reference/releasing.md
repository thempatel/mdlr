# Releasing mdlr

Releases are cut **from a laptop, no CI**. GoReleaser cross-compiles every
target with `cargo-zigbuild` (Rust) and the native Go toolchain, packages both
binaries together, publishes a GitHub Release on `thempatel/mdlr`, and pushes a
Homebrew **Cask** to `thempatel/homebrew-tap` for macOS.

## What ships

Each release produces, per platform, a `.tar.gz` containing **both** binaries
side by side:

- `mdlr` — the Rust binary (rust/ts/py extractors linked in)
- `mdlr-extract-go` — the Go extractor, spawned at runtime as a sibling of `mdlr`

Targets and how they're installed:

| OS    | Arch   | libc  | Install path                          |
| ----- | ------ | ----- | ------------------------------------- |
| macOS | arm64  | —     | `brew install thempatel/tap/mdlr`     |
| macOS | x86_64 | —     | `brew install thempatel/tap/mdlr`     |
| Linux | arm64  | glibc | Release tarball (manual / mise / aqua)|
| Linux | x86_64 | glibc | Release tarball (manual / mise / aqua)|
| Linux | arm64  | musl  | Release tarball (Alpine / distroless) |
| Linux | x86_64 | musl  | Release tarball (Alpine / distroless) |

macOS is distributed via a Homebrew Cask. Linux is **not** installed through
Homebrew (casks are macOS-only) — Linux users download the Release tarball
directly or via a tool like `mise` / `aqua`. The musl builds are static,
self-contained binaries for Alpine / distroless / portable use; the glibc builds
are dynamically linked.

> **Tap migration note:** the old hand-written `Formula/mdlr.rb` is superseded by
> the generated `Casks/mdlr.rb`. Delete `Formula/mdlr.rb` from the tap once the
> first cask release lands, or `brew` may resolve the stale formula.

## One-time setup

```sh
# Toolchain (pinned in mise.toml)
mise install                       # zig, cargo-zigbuild, goreleaser

# Rust cross-compile targets
rustup target add \
  x86_64-apple-darwin aarch64-apple-darwin \
  x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu \
  x86_64-unknown-linux-musl aarch64-unknown-linux-musl
```

You also need `gh` logged in (`gh auth status`) — it owns the credential, so
there is no PAT to create or store.

## Cut a release

```sh
# 1. Bump the workspace version and tag to match. The tag is the source of
#    truth; GoReleaser injects it into the binary via MDLR_VERSION (build.rs),
#    so `mdlr --version` will report the tag.
#    Edit `version` in Cargo.toml, then:
git commit -am "chore: release v0.2.0"
git tag v0.2.0

# 2. Release. Two non-obvious requirements, both handled on this line:
#    - ulimit -n: the ~250-rlib link step overruns macOS's default 256-fd limit.
#    - gh auth token: mints an ephemeral token from your gh login (repo scope),
#      which both creates the GitHub Release and pushes the cask to the tap.
ulimit -n 65536
GITHUB_TOKEN=$(gh auth token) goreleaser release --clean
```

> **Why `ulimit -n 65536`?** zig's linker opens every `.rlib` in mdlr's large
> dependency graph at once. With the macOS default of 256 file descriptors the
> link fails with `ProcessFdQuotaExceeded`. The limit must be raised in the
> shell that launches `goreleaser` — it cannot be a GoReleaser hook, because the
> linker runs in a child process that inherits the parent's limit.

## Dry run (no publish)

Validate the whole matrix and archive layout locally without touching GitHub:

```sh
ulimit -n 65536
goreleaser release --snapshot --clean
ls dist/                       # archives + checksums
cat dist/homebrew/Casks/mdlr.rb  # inspect the generated cask
```

## Troubleshooting

- **`ProcessFdQuotaExceeded` during linking** — you forgot `ulimit -n 65536`.
- **`mdlr --version` shows the wrong version** — the git tag and the build
  disagree. GoReleaser derives the version from the tag; make sure you tagged
  after bumping `Cargo.toml` and that the tag is reachable from `HEAD`.
- **macOS "killed: 9" on Apple Silicon** — a darwin binary wasn't ad-hoc signed.
  The `codesign --sign -` post-build hook in `.goreleaser.yaml` handles this;
  confirm it ran (it is a no-op on Linux targets).
- **Cask push rejected** — `gh auth token` lacks `repo` scope, or you are not
  a collaborator on `thempatel/homebrew-tap`. Check `gh auth status`.
