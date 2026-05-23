# mdlr

`mdlr` is a feedback tool that helps coding agents write less slop. Over many edit cylces, agents tend to write code that is incoherent, difficult to read, and bloated with
tests that are tautological or mock out the actual functionality that needs to be tested.

`mdlr` scans the codebase and hands coding agents a ranked list of the worst offenders — concrete, named symbols with a metric and a severity bucket — so it knows exactly what to clean up next. The loop is: make a batch of edits, run `mdlr check`, fix the top of the list, run it again, and repeat until satisfactory.

Supports Rust, Python, TypeScript, and Go and uses standard meatures of software "quality" alongside code coverage.

**Caveat**: This tool makes no claims on its ability to help agents write archictecturally sound code. The goal is to help agents write clean code so that it is easier for humans to read and get up to speed.

## Installation

```bash
brew tap thempatel/tap
brew install mdlr
```

## Quick Start

`mdlr` is built for coding agents. To kick off an improvement pass, tell your agent:

> Run `mdlr prompt` and follow the instructions.

`mdlr prompt` prints a markdown brief that walks the agent through running `mdlr check`, reading the ranked output, picking the worst offenders, and verifying its fixes. The agent works the list from there — you just review the diffs.

On a feature branch `mdlr check` analyzes only the files you've changed vs. `main`. On `main`/`master` it analyzes the whole repo. The first run extracts everything; later runs reuse a cache in `.mdlr/` and only re-extract files that changed.

If you want to see what the agent will be working with, run it yourself:

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

Each row is a refactor candidate: the metric that flagged it, the fully-qualified symbol, the measured value, and a severity bucket.

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

## Configuration

Defaults are tuned for typical codebases. If you want stricter or looser thresholds, drop a `.mdlr/config.yaml` in your project root and override just the values you care about — everything else keeps its default.

```yaml
thresholds:
  # Stricter function-size budget than the default 20/50/100/200
  function_size:
    excellent: 15
    good: 40
    fair: 80
    poor: 150

  # Looser cyclomatic threshold for a codebase with lots of validators
  cyclomatic:
    excellent: 8
    good: 15
    fair: 25
    poor: 40
```

Each field is the upper bound of its bucket — a value below `excellent` is "excellent", at or above `poor` is "critical". Lower-is-worse metrics like `line_cov` invert this (the field is the *low* boundary of each bucket). See [Configuration](reference/config.md) for the full schema covering every metric, display modes, and hub/CPD knobs.

## Documentation

### Metrics

- [Overview](metrics/overview.md) — summary of every metric, bucket labels, and reading fan-in × fan-out together
- [DAG Density](metrics/dag-density.md) — how connected the dependency graph is overall
- [Fan-In](metrics/fan-in.md) — how many units depend on each unit
- [Fan-Out](metrics/fan-out.md) — how many units each unit depends on
- [Complexity](metrics/complexity.md) — function size, parameters, cyclomatic complexity, max scope
- [Cognitive Complexity](metrics/cognitive-complexity.md) — nesting-aware complexity metric
- [File LOC](metrics/file-loc.md) — lines of code per file
- [Impl Metrics](metrics/impl-metrics.md) — methods per struct, LCOM
- [Line Coverage](metrics/line-coverage.md) — per-function test coverage from LCOV
- [Uncovered Branches](metrics/uncov-branches.md) — per-function untaken branches from LCOV

### Reference

- [CLI Commands](reference/cli.md)
- [Configuration](reference/config.md)

## License

MIT
