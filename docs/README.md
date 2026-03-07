# mdlr Documentation

A modularity analyzer for code. Uses the Rust compiler's HIR (High-level Intermediate Representation) to parse source files with full type resolution, builds a graph of code units and their relationships, and computes metrics for cohesion/coupling analysis.

## Documentation

### Getting Started

- [Installation](getting-started/installation.md)
- [Quick Start](getting-started/quick-start.md)

### Metrics

- [Overview](metrics/overview.md)
- [DAG Density](metrics/dag-density.md)
- [Fan-In](metrics/fan-in.md)
- [Fan-Out](metrics/fan-out.md)
- [Complexity](metrics/complexity.md) - function size, parameters, cyclomatic complexity, max scope
- [File LOC](metrics/file-loc.md) - lines of code per file
- [Impl Metrics](metrics/impl-metrics.md) - methods per impl, traits per type, LCOM
- [Interpreting Results](metrics/interpreting-results.md)

### Reference

- [CLI Commands](reference/cli.md)
- [Configuration](reference/config.md)
- [Graph Structure](reference/graph.md)
- [Supported Languages](reference/languages.md)
- [Cache System](reference/cache.md)
- [HIR Extractor](reference/hir-extract.md) - compiler-based Rust extraction with full type resolution

### Roadmap

- [Ignored Metrics](roadmap/ignored-metrics.md) - false positives and accepted design decisions

## License

MIT
