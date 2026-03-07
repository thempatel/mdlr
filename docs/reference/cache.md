# Cache System

mdlr uses a local `.mdlr/` directory to cache extraction results for use by `ls` and `get` commands.

## Cache Directory Structure

```
.mdlr/
├── index.json              # File metadata index
└── cache/
    └── <source-structure>/ # Mirrors source tree
        └── file.json       # Cached extraction per file
```

## How It Works

### Extraction

`mdlr check` always extracts all files from the workspace using `mdlr-extract-rust`. There is no incremental/change-detection step — every `check` invocation processes the full codebase and writes results to the cache.

### Per-File Cache Entry

Each source file has a corresponding cache file:

| Field | Description |
|-------|-------------|
| `source_path` | Relative path to source file |
| `units` | Extracted code units |
| `cached_at` | When cache was created |

Example: `src/main.rs` → `.mdlr/cache/src/main.json`

## Commands

### Run Analysis

```bash
# Analyze all files and save results to cache
mdlr check
```

## Gitignore Integration

mdlr respects `.gitignore` patterns. Files ignored by git are not analyzed or cached.

## Cache Location

The cache is stored in the project directory under `.mdlr/`. Add this to your `.gitignore`:

```
.mdlr/
```

## Manual Cache Operations

### Clear Cache

```bash
rm -rf .mdlr/
```

### Inspect Cache

```bash
# View cached file
cat .mdlr/cache/src/main.json | jq .
```
