# Metrics Overview

mdlr computes structural metrics that help you understand the modularity and coupling characteristics of your codebase.

## Available Metrics

### Structural Metrics

| Metric | Description |
|--------|-------------|
| [DAG Density](dag-density.md) | How connected the dependency graph is relative to a minimal tree |
| [Fan-In](fan-in.md) | How many units depend on each unit |
| [Fan-Out](fan-out.md) | How many units each unit depends on |

### Complexity Metrics

| Metric | Description |
|--------|-------------|
| [Function Size](complexity.md#function-size) | Lines of code per function |
| [Parameter Count](complexity.md#parameter-count) | Number of parameters per function |
| [Cyclomatic Complexity](complexity.md#cyclomatic-complexity) | Number of decision paths through a function |
| [Cognitive Complexity](cognitive-complexity.md) | Nesting-aware complexity that penalizes deeply nested code |
| [Max Scope Lines](complexity.md#max-scope-lines) | Largest single scope block within a function |

### File Metrics

| Metric | Description |
|--------|-------------|
| [File LOC](file-loc.md) | Lines of code per file |

### Impl Metrics

| Metric | Description |
|--------|-------------|
| [Methods per Struct](impl-metrics.md#methods-per-struct) | Number of methods defined on each struct |
| [LCOM4](impl-metrics.md#lcom4-lack-of-cohesion-of-methods) | Lack of Cohesion of Methods — connected components of related methods |

### Coverage Metrics

Computed when `mdlr check --cov <file>` is run with an LCOV coverage file.

| Metric | Description |
|--------|-------------|
| [Line Coverage](line-coverage.md) | Per-function % of attributed DA lines that ran at least once |
| [Uncovered Branches](uncov-branches.md) | Per-function count of BRDA records that were never taken |

## How Metrics Are Computed

1. **Graph extraction**: Source files are parsed using the Rust compiler's HIR to identify code units (functions, structs, traits, etc.)

2. **Edge detection**: Relationships between units are identified (calls, reads, writes)

3. **Metric computation**: Structural metrics are calculated from the graph

## Using Metrics

Metrics are most useful when:

- **Comparing modules**: Which parts of the codebase are most interconnected?
- **Identifying hotspots**: Which units are critical hubs?
- **Guiding refactoring**: Where should you focus decoupling efforts?

## Bucket Labels

Every metric value is paired with one of five buckets so you can quickly assess severity without remembering the raw thresholds:

| Bucket | Meaning |
|--------|---------|
| excellent | Well within healthy range |
| good | Healthy, no action needed |
| fair | Acceptable, consider monitoring |
| poor | Action recommended |
| critical | Requires attention |

Thresholds for each bucket are tunable per metric — see [Configuration](../reference/config.md).

## Tracking Trends Over Time

For a longitudinal view of code health, append a JSON snapshot on a regular cadence:

```bash
# Weekly snapshot
mdlr check -A --format json >> metrics-history.jsonl
```

Watch for:

- DAG density creeping upward (increasing coupling)
- New hub units appearing (centralization)
- Fan-out max increasing (growing god functions)
