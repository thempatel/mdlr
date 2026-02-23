# HIR Extractor

`mdlr-extract-rust` is a standalone binary that uses cargo-as-library and the Rust compiler's HIR (High-level Intermediate Representation) to extract code units with fully-resolved type information.

## Requirements

- Nightly Rust toolchain with `rustc-dev` and `llvm-tools` components
- The crate's `rust-toolchain.toml` handles this automatically when building from its directory

## Usage

The binary is invoked directly by `mdlr check`:

```bash
mdlr-extract-rust --manifest-path path/to/Cargo.toml --output output.json
```

### CLI Arguments

| Argument | Description |
|----------|-------------|
| `--manifest-path` | Path to the workspace `Cargo.toml` |
| `--output` | Output directory for per-file JSON results (mirrors source tree structure) |
| `--package` | (Optional, repeatable) Package names to extract from. Defaults to all workspace members. |

## Output Format

The output directory mirrors the source tree. Each source file gets a corresponding JSON file:

```
<output>/
└── crates/
    └── mdlr-core/
        └── src/
            └── graph/
                └── types.json    # from crates/mdlr-core/src/graph/types.rs
```

Each JSON file contains a single `FileCacheEntry`:

```json
{
  "source_path": "crates/mdlr-core/src/graph/types.rs",
  "units": [
    {
      "id": "graph::types::Span",
      "kind": "Struct",
      "file": "crates/mdlr-core/src/graph/types.rs",
      "span": { "start_line": 5, "start_col": 0, "end_line": 10, "end_col": 1 },
      "reads": [],
      "writes": [],
      "calls": [],
      "tags": [],
      "params": 0,
      "branches": 0
    }
  ],
  "cached_at": 1769900625
}
```

## Building

```bash
cd crates/mdlr-extract-rust
cargo build
```

The `rust-toolchain.toml` ensures the correct nightly toolchain is used automatically.

## ID Format

Unit IDs are module-relative within the crate (matching `def_path_str` output), e.g. `graph::types::Span`.
