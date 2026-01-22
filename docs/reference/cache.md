# Cache System

mdlr uses a local `.mdlr/` directory to cache extraction results, enabling fast incremental analysis.

## Cache Directory Structure

```
.mdlr/
├── index.json              # File metadata index
└── cache/
    └── <source-structure>/ # Mirrors source tree
        └── file.json       # Cached extraction per file
```

## How It Works

### Change Detection

mdlr uses mtime (modification time) and file size to detect changes:

1. On `analyze`, each source file's mtime/size is compared with the cached value
2. If different or missing, the file is re-extracted
3. If unchanged, cached units are loaded directly

### Per-File Cache Entry

Each source file has a corresponding cache file:

| Field | Description |
|-------|-------------|
| `source_path` | Relative path to source file |
| `mtime` | File modification time (UNIX epoch seconds) |
| `size` | File size in bytes |
| `units` | Extracted code units |
| `cached_at` | When cache was created |

Example: `src/main.rs` → `.mdlr/cache/src/main.json`

### Project Index

The `index.json` file tracks all known files:

| Field | Description |
|-------|-------------|
| `version` | Schema version for migrations |
| `files` | Map of path → {mtime, size} |
| `last_scan` | Timestamp of last full scan |

## Commands

### Check What Needs Analysis

```bash
# Show new/changed files
mdlr todo

# Also show files with untagged units
mdlr todo --all
```

### Run Analysis

```bash
# Incremental analysis (only changed files)
mdlr analyze

# Force re-analysis of all files
mdlr analyze --force
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
# View index
cat .mdlr/index.json | jq .

# View cached file
cat .mdlr/cache/src/main.json | jq .
```

## Performance

For a typical project:

- First run: All files extracted (slower)
- Subsequent runs: Only changed files re-extracted (fast)
- Cache lookup: O(1) per file using mtime/size comparison
