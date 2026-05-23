# CLI Reference

## Global Options

| Option | Description |
|--------|-------------|
| `-h, --help` | Print help |
| `-V, --version` | Print version |

## Commands

### check

Run analysis and display metrics.

```bash
mdlr check [target] [-k <count>] [--pretty] [--format <format>] [-A] [-f <dir>] [-q] [--cov <path>]...
```

| Option | Default | Description |
|--------|---------|-------------|
| `target` | `.` | Path (file/directory) or fully qualified symbol ID to analyze |
| `-k` | `10` | Max opportunities to show per metric (-1 for all) |
| `--pretty` | false | Pretty print as aligned table |
| `--format` | `text` | Output format: `text` or `json` |
| `-A, --all` | false | Analyze all files even when on a branch |
| `-f, --filter` | - | Scope analysis to a specific directory (combines with diff/all mode) |
| `-q, --quiet` | false | Suppress progress display (progress is shown by default when stderr is a TTY) |
| `--cov <PATH>` | - | LCOV coverage file to overlay onto changed files. Repeatable: pass `--cov` once per file and they are merged. Adds two metrics: `line_cov` (per-function %) and `uncov_branches` (per-function untaken-branch count, only when the lcov has BRDA records). See [line coverage](../metrics/line-coverage.md) and [uncovered branches](../metrics/uncov-branches.md). |

By default, `check` uses **diff mode** on branches (only analyzing files changed since main/master) and analyzes all files when on main/master. Use `-A` to force analyzing all files when on a branch. Use `-f` to scope metrics to a specific directory — this works in both diff and all modes.

Running `check` extracts all files and writes results to the cache.

**Examples:**

```bash
# Analyze (diff mode on branches, all files on main/master)
mdlr check

# Force all files even when on a branch
mdlr check -A

# Analyze specific directory
mdlr check ./src/metrics

# Analyze specific file
mdlr check ./src/main.rs

# Analyze a specific function
mdlr check "src/main.rs::handle_check"

# Analyze a method in an impl block
mdlr check "src/cache/store.rs::impl CacheStore::load_entry"

# Analyze an impl block
mdlr check "src/cache/store.rs::impl CacheStore"

# Scope to a directory (all mode)
mdlr check -A -f src/metrics

# Scope to a directory (diff mode on branches)
mdlr check -f src/metrics

# Show all opportunities (not just top 10)
mdlr check -k -1

# Pretty-printed table output
mdlr check --pretty

# JSON output for scripting
mdlr check --format json

# Overlay coverage from one LCOV file
mdlr check --cov target/llvm-cov/lcov.info

# Merge multiple LCOV files (e.g. frontend + backend)
mdlr check --cov frontend/coverage/lcov.info --cov backend/lcov.info
```

---

### ls

List symbols (units) in a file or directory.

```bash
mdlr ls [path] [--kind <kind>] [--format <format>]
```

| Option | Default | Description |
|--------|---------|-------------|
| `path` | `.` | Directory or file to list symbols from |
| `--kind` | - | Filter by unit kind: `function`, `struct`, `trait`, `impl`, `module` |
| `--format` | `text` | Output format: `text` or `json` |

**Examples:**

```bash
# List all symbols in current directory
mdlr ls

# List only functions
mdlr ls --kind function

# List symbols from specific directory
mdlr ls ./src

# JSON output for scripting
mdlr ls --format json
```

**Output columns:**

| Column | Description |
|--------|-------------|
| ID | Unique symbol identifier |
| Kind | Type of unit (Function, Struct, etc.) |
| File | Source file path |
| Start-End | Line number range |

---

### get

Get the content of a symbol.

```bash
mdlr get <symbol> [--format <format>]
```

| Option | Default | Description |
|--------|---------|-------------|
| `symbol` | required | Symbol ID to retrieve (from `mdlr ls`) |
| `--format` | `text` | Output format: `text` or `json` |

**Examples:**

```bash
# Get a function's source code
mdlr get compute

# Get symbol as JSON
mdlr get compute --format json
```
