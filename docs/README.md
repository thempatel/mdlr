# mdlr Documentation

A modularity analyzer for code. Uses tree-sitter to parse source files, builds an intermediate representation (IR) graph of code units and their relationships, and computes metrics for cohesion/coupling analysis.

## Documentation

### Getting Started

- [Installation](getting-started/installation.md)
- [Quick Start](getting-started/quick-start.md)

### Metrics

- [Overview](metrics/overview.md)
- [DAG Density](metrics/dag-density.md)
- [Fan-In](metrics/fan-in.md)
- [Fan-Out](metrics/fan-out.md)
- [Complexity](metrics/complexity.md) - function size, parameters, cyclomatic complexity
- [Impl Metrics](metrics/impl-metrics.md) - methods per impl, traits per type, LCOM
- [Tag Coverage](metrics/tag-coverage.md)
- [Interpreting Results](metrics/interpreting-results.md)

### Reference

- [CLI Commands](reference/cli.md)
- [Configuration](reference/config.md)
- [Graph Structure](reference/graph.md)
- [Supported Languages](reference/languages.md)
- [Cache System](reference/cache.md)

## License

MIT
