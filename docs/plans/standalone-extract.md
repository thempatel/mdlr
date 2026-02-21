# Plan: Make mdlr-extract-rust Standalone (No Cargo CLI)

## Goal

Replace the current `cargo check` + `RUSTC_WRAPPER` architecture with a fully standalone `mdlr-extract-rust` binary that can analyze any Rust project without shelling out to cargo. It will parse manifests, resolve dependencies, compile them from source using `rustc_driver`, and extract HIR data from target crates.

## Current Architecture

```
mdlr CLI
  → cargo metadata (discover packages)
  → cargo check -p <pkg> (with RUSTC_WRAPPER=mdlr-extract-rust)
    → cargo compiles deps normally
    → cargo invokes mdlr-extract-rust as wrapper for target crate
      → mdlr-extract-rust detects target crate via --crate-name + MDLR_HIR_CRATE
      → calls rustc_driver::run_compiler() with after_analysis callback
      → writes FileCacheEntry JSON
```

## New Architecture

```
mdlr CLI
  → mdlr-extract-rust --manifest-path Cargo.toml --mapping mapping.json [--package <pkg>]
    → parses Cargo.toml + Cargo.lock
    → resolves dependency graph + features
    → finds dep sources in ~/.cargo/registry/src/ or local paths
    → compiles all deps in topological order via rustc_driver (--emit=metadata)
    → compiles proc-macro deps via rustc_driver (--emit=link, --crate-type=proc-macro)
    → runs build scripts, parses their output
    → compiles target crate(s) with HirExtractCallbacks
    → writes FileCacheEntry JSON
```

## Key Files to Modify

- **`crates/mdlr-extract-rust/src/main.rs`** — New standalone entry point, CLI arg parsing, orchestration
- **`crates/mdlr-extract-rust/Cargo.toml`** — Add deps: `toml`, `clap`, `tempfile`
- **`crates/mdlr/src/main.rs`** (lines 397-541) — Replace `extract_rust()`, remove cargo shelling

## New Files to Create

```
crates/mdlr-extract-rust/src/
  ├── manifest.rs       # Cargo.toml parsing (package, deps, features, workspace, targets)
  ├── lockfile.rs       # Cargo.lock parsing (pinned versions, sources, checksums)
  ├── resolve.rs        # Dependency graph construction, feature resolution, topo sort
  ├── source.rs         # Find crate source dirs (registry, path deps, workspace members)
  ├── compile.rs        # Construct rustc args, invoke rustc_driver for each dep
  ├── build_script.rs   # Compile + execute build.rs, parse cargo directives
  └── pipeline.rs       # Top-level orchestration: parse → resolve → compile → extract
```

Existing HIR extraction files (`visitor.rs`, `calls.rs`, `branches.rs`, `field_access.rs`) remain unchanged.

## Implementation Phases

### Phase 1: Manifest Parsing (`manifest.rs`)

Parse `Cargo.toml` into structured types using `toml` + `serde`:

```rust
struct Manifest {
    package: Option<Package>,           // name, version, edition, build
    lib: Option<Target>,                // path, name, crate-type, proc-macro
    bin: Vec<Target>,                   // binary targets
    dependencies: BTreeMap<String, Dep>,
    dev_dependencies: BTreeMap<String, Dep>,
    build_dependencies: BTreeMap<String, Dep>,
    features: BTreeMap<String, Vec<String>>,
    workspace: Option<Workspace>,       // members, exclude, dependencies
}

struct Dep {
    version: Option<String>,
    path: Option<PathBuf>,
    features: Vec<String>,
    default_features: bool,            // default: true
    optional: bool,
    package: Option<String>,           // rename: package = "real-name"
}
```

Handle workspace inheritance (`workspace = true` in dep specs, `edition.workspace = true`, etc.).

### Phase 2: Lock File Parsing (`lockfile.rs`)

Parse `Cargo.lock` (v3/v4 format):

```rust
struct Lockfile {
    packages: Vec<LockedPackage>,
}

struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,            // "registry+https://..." or "path+file://..."
    checksum: Option<String>,
    dependencies: Vec<String>,         // "name version source" format
}
```

The lock file is the **already-resolved** dependency graph. We don't need to re-solve — just read it.

### Phase 3: Source Discovery (`source.rs`)

Locate crate source directories:

1. **Registry crates**: `~/.cargo/registry/src/index.crates.io-<hash>/<name>-<version>/`
   - Enumerate `~/.cargo/registry/src/` to find the index directory name (varies by system)
   - Match by `<name>-<version>`

2. **Path dependencies**: Resolve relative to the manifest that declares them

3. **Workspace members**: Resolve from workspace root using `workspace.members` globs

4. **Git dependencies**: `~/.cargo/git/checkouts/<name>-<hash>/<short-hash>/`
   - Lower priority for MVP; error gracefully

```rust
fn find_source(pkg: &LockedPackage, workspace_root: &Path) -> Result<PathBuf>
```

### Phase 4: Dependency Graph & Feature Resolution (`resolve.rs`)

Build a compilation-ready dependency graph:

1. **Build graph from lock file**: Each `LockedPackage` → node, `dependencies` → edges
2. **Load manifests**: For each package, parse its Cargo.toml from the source directory
3. **Feature propagation**: Starting from the root crate's enabled features, propagate through the graph:
   - Default features (unless `default-features = false`)
   - Explicitly requested features
   - `dep:foo` syntax (optional dep activation)
   - Feature forwarding (`"serde" = ["dep:serde", "chrono/serde"]`)
4. **Topological sort**: Order for compilation (leaves first)
5. **Determine crate types**: `lib` (most deps), `proc-macro` (if `proc-macro = true`), `bin` (targets)

Output:
```rust
struct CompilePlan {
    units: Vec<CompileUnit>,  // in topological order
}

struct CompileUnit {
    name: String,
    version: String,
    source_dir: PathBuf,
    src_path: PathBuf,           // lib.rs, main.rs, etc.
    edition: String,
    crate_type: CrateType,      // Lib, ProcMacro, Bin
    features: Vec<String>,       // enabled features
    dependencies: Vec<DepRef>,   // (extern_name, package_id)
    build_script: Option<PathBuf>,
    cfgs: Vec<String>,           // from build scripts (populated later)
    is_target: bool,             // should we extract HIR from this?
}
```

### Phase 5: Compilation Pipeline (`compile.rs`)

For each `CompileUnit` in order, construct rustc args and compile:

```rust
fn compile_unit(unit: &CompileUnit, artifacts: &ArtifactStore, sysroot: &Path) -> Result<PathBuf> {
    let mut args = vec!["rustc".into(), unit.src_path.display().to_string()];

    args.extend(["--crate-name", &unit.name.replace('-', "_")]);
    args.extend(["--edition", &unit.edition]);

    match unit.crate_type {
        CrateType::Lib => {
            args.extend(["--crate-type", "lib"]);
            args.extend(["--emit", "metadata"]);  // only .rmeta, fast
        }
        CrateType::ProcMacro => {
            args.extend(["--crate-type", "proc-macro"]);
            args.extend(["--emit", "metadata,link"]);  // needs .so/.dylib
        }
        CrateType::Bin => { /* skip for analysis */ }
    }

    // Output directory for this crate's artifacts
    args.extend(["--out-dir", &artifacts.deps_dir().display().to_string()]);

    // Stable hash suffix for unique filenames
    args.extend(["-C", &format!("extra-filename=-{}", unit.hash())]);

    // Sysroot
    args.extend(["--sysroot", &sysroot.display().to_string()]);

    // Direct dependencies as --extern flags
    for dep in &unit.dependencies {
        let artifact_path = artifacts.get(&dep.package_id)?;
        args.extend(["--extern", &format!("{}={}", dep.extern_name, artifact_path.display())]);
    }

    // Search path for transitive deps
    args.extend(["-L", &format!("dependency={}", artifacts.deps_dir().display())]);

    // Feature cfg flags
    for feature in &unit.features {
        args.extend(["--cfg", &format!("feature=\"{}\"", feature)]);
    }

    // Build script cfg flags
    for cfg in &unit.cfgs {
        args.extend(["--cfg", cfg]);
    }

    // Compile
    rustc_driver::catch_fatal_errors(|| {
        rustc_driver::run_compiler(&args, &mut NoopCallbacks);
    })?;

    Ok(artifacts.artifact_path(unit))
}
```

**Artifact storage**: `.mdlr/build/deps/` directory. Each compiled crate produces `lib<name>-<hash>.rmeta` (and `.so` for proc macros). The hash is deterministic from (name, version, features) so artifacts can be cached across runs.

**Sysroot**: Determined at runtime. Since `mdlr-extract-rust` is compiled with nightly + rustc-dev, its default sysroot is the nightly toolchain. Can also detect via the `SYSROOT` env var or by inspecting the binary's rpath.

### Phase 6: Build Script Support (`build_script.rs`)

Build scripts (`build.rs`) are compiled as a separate binary, executed, and their stdout parsed for cargo directives.

1. **Compile**: `build.rs` → binary, using `rustc_driver` with `--crate-type=bin --emit=link`
   - Build script deps come from `[build-dependencies]`
2. **Execute**: Run the binary with env vars:
   - `CARGO_MANIFEST_DIR`, `OUT_DIR`, `TARGET`, `HOST`, `PROFILE=debug`
   - `CARGO_CFG_TARGET_*` vars, `CARGO_FEATURE_*` vars
   - `CARGO_PKG_*` vars (name, version, etc.)
3. **Parse stdout**: Line-oriented protocol:
   - `cargo::rustc-cfg=KEY` → add `--cfg KEY` to the crate's compilation
   - `cargo::rustc-env=KEY=VALUE` → set env var during compilation
   - `cargo::rustc-link-lib=LIB` → (skip for analysis, only needed for linking)
   - `cargo::rustc-link-search=PATH` → add to search paths
   - `cargo::rerun-if-changed=PATH` → (skip, no incremental for now)
   - `cargo::warning=MSG` → print warning

**MVP simplification**: Many build scripts only emit `rustc-cfg` directives. Handle the common directives first, warn on unrecognized ones.

### Phase 7: Target Crate Extraction (update `main.rs`)

For target crates (the ones in the user's project), compile with `HirExtractCallbacks` instead of `NoopCallbacks`. This is the existing extraction logic — unchanged.

```rust
fn extract_target(unit: &CompileUnit, artifacts: &ArtifactStore, mapping: &HashMap<String, String>) {
    let args = build_rustc_args(unit, artifacts); // same as compile_unit
    let mut callbacks = HirExtractCallbacks { mapping: mapping.clone() };

    rustc_driver::catch_fatal_errors(|| {
        rustc_driver::run_compiler(&args, &mut callbacks);
    });
}
```

### Phase 8: CLI Interface (update `main.rs`)

New standalone interface for `mdlr-extract-rust`:

```
mdlr-extract-rust \
  --manifest-path <path/to/Cargo.toml> \
  --mapping <path/to/mapping.json> \
  [--package <name>]              # which package(s) to extract (default: all workspace members)
  [--build-dir <path>]            # where to store compiled artifacts (default: .mdlr/build)
  [--features <f1,f2>]            # additional features to enable
  [--no-default-features]
```

Backward compat: detect RUSTC_WRAPPER mode if `args[1]` doesn't start with `--` (legacy path).

### Phase 9: Update mdlr CLI (`crates/mdlr/src/main.rs`)

Replace `extract_rust()` (lines 475-541):

```rust
fn extract_rust(files: &[FileCacheEntry], workspace_root: &Path) -> Result<Vec<Option<FileCacheEntry>>> {
    let tmp_dir = tempfile::tempdir()?;
    // Build mapping (same as current)
    let mapping = build_mapping(files, workspace_root, &tmp_dir)?;
    let mapping_path = tmp_dir.path().join("mapping.json");
    std::fs::write(&mapping_path, serde_json::to_string(&mapping)?)?;

    let extract_bin = find_extract_rust_binary()?;

    // Single invocation — mdlr-extract-rust handles everything
    let output = Command::new(&extract_bin)
        .arg("--manifest-path").arg(workspace_root.join("Cargo.toml"))
        .arg("--mapping").arg(&mapping_path)
        .output()?;

    // Load results (same as current)
    load_results(&tmp_dir, files.len())
}
```

Remove `discover_packages_for_files()` — no longer needed.

## Phasing & Priority

| Phase | Description | Complexity | Notes |
|-------|-------------|-----------|-------|
| 1 | Manifest parsing | Medium | Foundation for everything |
| 2 | Lock file parsing | Low | Straightforward TOML |
| 3 | Source discovery | Medium | Registry path heuristics |
| 4 | Dep graph + features | High | Feature propagation is tricky |
| 5 | Compilation pipeline | High | Core of the work |
| 6 | Build scripts | High | Many edge cases |
| 7 | Target extraction | Low | Reuses existing code |
| 8 | CLI interface | Low | Arg parsing |
| 9 | mdlr CLI update | Low | Simplifies existing code |

**MVP order**: 1 → 2 → 3 → 4 (basic) → 5 → 7 → 8 → 9. Add Phase 6 (build scripts) and advanced Phase 4 (full feature resolution) iteratively.

**MVP limitations** (to address later):
- No build script support (many crates don't need it; warn when detected)
- No git dependencies (only registry + path)
- No custom registries
- Feature resolution handles common cases (default features, explicit features)
- No incremental compilation caching (compile everything fresh; fast since `--emit=metadata`)

## Verification

1. **Unit tests**: For manifest parsing, lock file parsing, source discovery, graph construction
2. **Integration test**: Run `mdlr-extract-rust --manifest-path Cargo.toml --mapping test-mapping.json` on this workspace (mdlr itself) and verify it produces the same `FileCacheEntry` output as the current RUSTC_WRAPPER approach
3. **Cross-project test**: Run on a simple external project (e.g., a project with serde, clap deps) to verify registry source discovery works
4. **End-to-end**: Run `mdlr check` with the new pipeline and compare output to the old pipeline

## Dependencies to Add

```toml
# crates/mdlr-extract-rust/Cargo.toml
[dependencies]
toml = "0.8"           # Cargo.toml + Cargo.lock parsing
clap = { version = "4", features = ["derive"] }  # CLI args
tempfile = "3"         # temp dirs for compilation artifacts
home = "0.5"           # ~/.cargo discovery (cross-platform)
```

## Risks

1. **rustc_driver process model**: `run_compiler()` is designed to be called once per process (global state, allocators). Compiling multiple crates in one process may require `catch_fatal_errors` + careful state management, or forking. **Mitigation**: If in-process compilation of multiple crates fails, fork a subprocess per compilation unit (still using our own binary, not cargo).

2. **Proc macro host compilation**: Proc macros must produce host-platform dylibs. This requires full codegen, not just metadata. **Mitigation**: Detect proc-macro crates and compile with `--emit=link`.

3. **Build script complexity**: Some build scripts run arbitrary code (bindgen, cc, protobuf). We may not perfectly emulate cargo's build script environment. **Mitigation**: Implement common directives first, warn on unsupported ones, provide a `--use-cargo-for-deps` fallback flag.

4. **Feature resolution completeness**: Cargo's feature resolver v2 is complex (platform-specific features, weak dependencies, `dep:` syntax). **Mitigation**: Start with the common cases, expand coverage with test-driven development.
