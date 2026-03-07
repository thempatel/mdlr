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
| [Max Scope Lines](complexity.md#max-scope-lines) | Largest single scope block within a function |

### File Metrics

| Metric | Description |
|--------|-------------|
| [File LOC](file-loc.md) | Lines of code per file |

### Impl Metrics

| Metric | Description |
|--------|-------------|
| [Methods per Impl](impl-metrics.md#methods-per-impl) | Number of methods in each impl block |
| [Traits per Type](impl-metrics.md#traits-per-type) | How many traits each type implements |
| [LCOM](impl-metrics.md#lcom-lack-of-cohesion-of-methods) | Lack of Cohesion of Methods - how related methods are |

## How Metrics Are Computed

1. **Graph extraction**: Source files are parsed using the Rust compiler's HIR to identify code units (functions, structs, traits, etc.)

2. **Edge detection**: Relationships between units are identified (calls, reads, writes)

3. **Metric computation**: Structural metrics are calculated from the graph

## Using Metrics

Metrics are most useful when:

- **Tracking trends over time**: Is coupling increasing or decreasing?
- **Comparing modules**: Which parts of the codebase are most interconnected?
- **Identifying hotspots**: Which units are critical hubs?
- **Guiding refactoring**: Where should you focus decoupling efforts?

See [Interpreting Results](interpreting-results.md) for guidance on what the numbers mean.
