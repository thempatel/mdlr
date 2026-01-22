# CLI Reference

## Global Options

| Option | Description |
|--------|-------------|
| `-h, --help` | Print help |
| `-V, --version` | Print version |

## Commands

### todo

Show files that need analysis.

```bash
mdlr todo [path] [--all] [--format <format>]
```

| Option | Default | Description |
|--------|---------|-------------|
| `path` | `.` | Directory to check |
| `--all` | false | Also show files with untagged units |
| `--format` | `text` | Output format: `text` or `json` |

**Examples:**

```bash
# Check current directory
mdlr todo

# Check specific directory
mdlr todo ./src

# Include files with untagged units
mdlr todo --all

# JSON output for scripting
mdlr todo --format json
```

---

### analyze

Run analysis on a directory and display metrics.

```bash
mdlr analyze [path] [--force] [--format <format>]
```

| Option | Default | Description |
|--------|---------|-------------|
| `path` | `.` | Directory to analyze |
| `--force` | false | Force re-analysis of all files |
| `--format` | `text` | Output format: `text` or `json` |

**Examples:**

```bash
# Analyze current directory (incremental)
mdlr analyze

# Analyze specific directory
mdlr analyze ./my-project

# Force full re-analysis
mdlr analyze --force

# JSON output for scripting
mdlr analyze --format json
```

---

### export

Export the graph from cached analysis.

```bash
mdlr export [path] [--format <format>]
```

| Option | Default | Description |
|--------|---------|-------------|
| `path` | `.` | Directory to export from |
| `--format` | `json` | Output format: `text` or `json` |

**Examples:**

```bash
# Export as JSON
mdlr export > graph.json

# Export specific directory
mdlr export ./my-project --format json

# Human-readable list
mdlr export --format text
```
