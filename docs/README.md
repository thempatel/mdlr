# mdlr

`mdlr` scans a codebase and points you at the parts that are most likely to be a pain to work with — oversized functions, deeply nested logic, types with too many responsibilities, copy-pasted blocks, and (optionally) code that isn't covered by tests. It works on Rust, Python, TypeScript, and Go.

The output is a ranked list of concrete refactor targets. Pick the worst offenders, fix them, run it again.

## Installation

```bash
brew tap thempatel/tap
brew install mdlr
```

## Quick Start

From the root of your project, run:

```bash
mdlr check
```

On a feature branch this analyzes only the files you've changed vs. `main`. On `main`/`master` it analyzes the whole repo. The first run extracts everything; later runs reuse a cache in `.mdlr/` and only re-extract files that changed.

Output looks like this:

```
$ mdlr check --pretty
metric         symbol                                       value  bucket
function_size  mdlr_extract_py::tokenizer::tokenize_py      346    critical
function_size  mdlr_extract_ts::tokenizer::tokenize_ts      295    critical
params         mdlr::check::handle_check                    10     critical
cyclomatic     mdlr_extract_py::tokenizer::tokenize_py      102    critical
cognitive      mdlr_extract_py::tokenizer::tokenize_py      199    critical
cognitive      mdlr_metrics::coverage::CoverageMetrics::compute 71  critical
```

Each row is a refactor candidate: the metric that flagged it, the fully-qualified symbol, the measured value, and a severity bucket. Start at the top.

### Common flags

```bash
# Force analysis of all files (override the branch-only default)
mdlr check -A

# Scope to a directory or single file
mdlr check src/metrics
mdlr check src/main.rs

# Limit a full-repo run to a sub-tree
mdlr check -A -f src/metrics

# Drill into a specific symbol
mdlr check "mdlr::check::handle_check"

# Show more results per metric (default 10)
mdlr check -k 25

# Overlay test coverage from an LCOV file — adds line_cov and uncov_branches
mdlr check --cov target/llvm-cov/lcov.info

# Merge coverage from multiple LCOV files (monorepo with frontend + backend)
mdlr check --cov frontend/lcov.info --cov backend/lcov.info
```

### Exploring metrics and symbols

```bash
# List every metric with a one-line description
mdlr metrics ls

# Get details and threshold buckets for a single metric
mdlr metrics get cognitive

# List symbols in a file or directory
mdlr ls src/metrics

# Print the source of one symbol
mdlr get "mdlr::check::handle_check"
```

### Cache

Analysis results are cached in `.mdlr/` at the project root. Add it to your `.gitignore`:

```
.mdlr/
```

## Documentation

### Metrics

- [Overview](metrics/overview.md)
- [DAG Density](metrics/dag-density.md)
- [Fan-In](metrics/fan-in.md)
- [Fan-Out](metrics/fan-out.md)
- [Complexity](metrics/complexity.md) — function size, parameters, cyclomatic complexity, max scope
- [Cognitive Complexity](metrics/cognitive-complexity.md) — nesting-aware complexity metric
- [File LOC](metrics/file-loc.md) — lines of code per file
- [Impl Metrics](metrics/impl-metrics.md) — methods per impl, traits per type, LCOM
- [Line Coverage](metrics/line-coverage.md) — per-function test coverage from LCOV
- [Uncovered Branches](metrics/uncov-branches.md) — per-function untaken branches from LCOV
- [Interpreting Results](metrics/interpreting-results.md)

### Reference

- [CLI Commands](reference/cli.md)
- [Configuration](reference/config.md)
- [Graph Structure](reference/graph.md)
- [Supported Languages](reference/languages.md)
- [Cache System](reference/cache.md)
- [HIR Extractor](reference/hir-extract.md) — compiler-based Rust extraction with full type resolution

### Roadmap

- [Ignored Metrics](roadmap/ignored-metrics.md) — false positives and accepted design decisions

## License

MIT
