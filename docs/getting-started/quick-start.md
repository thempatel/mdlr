# Quick Start

## Basic Workflow

1. **Check what needs analysis:**
   ```bash
   mdlr todo
   ```

2. **Run analysis** to extract the graph and compute metrics:
   ```bash
   mdlr analyze
   ```

3. **Export the graph** for further processing:
   ```bash
   mdlr export --format json
   ```

## Example Session

```bash
# Check status (first time, all files are new)
$ mdlr todo
New files (7):
  src/main.rs
  src/lib.rs
  src/cli.rs
  src/graph/mod.rs
  src/graph/types.rs
  src/extract/mod.rs
  src/extract/rust.rs

Run 'mdlr analyze' to update 7 file(s).

# Run analysis
$ mdlr analyze
Analysis complete

Files: 7 extracted, 0 from cache
Graph: 87 units, 36 edges

Structural Metrics
==================

DAG Density: 0.419

Fan-In:  max=4, mean=0.43
Fan-Out: max=6, mean=0.43

Top Fan-Out:
  extract_from_node (6)
  main (4)
  build_graph (3)

Top Fan-In:
  get_node_name (4)
  node_span (4)
  compute (3)

# Check status again (all cached now)
$ mdlr todo
All files are up to date.

# Modify a file, then check again
$ echo "// comment" >> src/main.rs
$ mdlr todo
Changed files (1):
  src/main.rs

Run 'mdlr analyze' to update 1 file(s).

# Incremental analysis (only re-extracts changed files)
$ mdlr analyze
Analysis complete

Files: 1 extracted, 6 from cache
Graph: 87 units, 36 edges
...
```

## Cache Directory

Analysis results are cached in `.mdlr/` in your project root. Add this to your `.gitignore`:

```
.mdlr/
```

## Force Re-Analysis

To ignore the cache and re-analyze all files:

```bash
mdlr analyze --force
```
