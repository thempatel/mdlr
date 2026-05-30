# Configuration

mdlr supports optional configuration through a YAML file at `.mdlr/config.yaml`. The config file is searched recursively up from the current working directory, similar to `.gitignore`.

## File Location

Place your config file at `.mdlr/config.yaml` in your project root or any parent directory. mdlr searches upward from the current directory until it finds a config file or reaches the filesystem root.

```
project/
├── .mdlr/
│   └── config.yaml    # Config found here
├── src/
│   └── lib.rs
└── Cargo.toml
```

## Configuration Options

### Thresholds

Configure thresholds for each metric. Values below a threshold receive that bucket label. Values at or above the `poor` threshold are labeled `critical`.

```yaml
thresholds:
  # Structural metrics
  dag_density:
    excellent: 0.5
    good: 1.0
    fair: 1.5
    poor: 2.0

  fan_in_max:
    excellent: 3
    good: 5
    fair: 10
    poor: 15

  fan_in_mean:
    excellent: 0.5
    good: 1.0
    fair: 2.0
    poor: 3.0

  fan_out_max:
    excellent: 3
    good: 5
    fair: 8
    poor: 12

  fan_out_mean:
    excellent: 0.5
    good: 1.0
    fair: 2.0
    poor: 3.0

  # Complexity metrics
  function_size:
    excellent: 20
    good: 50
    fair: 100
    poor: 200

  params:
    excellent: 3
    good: 5
    fair: 7
    poor: 10

  cyclomatic:
    excellent: 5
    good: 10
    fair: 20
    poor: 30

  cognitive:
    excellent: 5
    good: 10
    fair: 15
    poor: 25

  max_scope:
    excellent: 15
    good: 30
    fair: 50
    poor: 100

  # File-level metrics
  file_loc:
    excellent: 200
    good: 400
    fair: 600
    poor: 1000

  duplication_pct:
    excellent: 3
    good: 5
    fair: 10
    poor: 20

  # Impl metrics
  methods_per_struct:
    excellent: 5
    good: 10
    fair: 15
    poor: 25

  # LCOM4 = connected components. 1 = cohesive, 2+ = should split.
  lcom:
    excellent: 2
    good: 3
    fair: 4
    poor: 5

  # Coverage metrics (only emitted when --cov is passed).
  # line_cov is lower-is-worse: each field is the LOW boundary of that bucket.
  line_cov:
    excellent: 90
    good: 80
    fair: 70
    poor: 60

  uncov_branches:
    excellent: 1
    good: 3
    fair: 6
    poor: 10
```

### Display Mode

Control how metric values are displayed:

```yaml
display:
  mode: both  # Options: "both", "label", "value"
```

| Mode | Output Example |
|------|----------------|
| `both` | `0.419 (excellent)` |
| `label` | `excellent` |
| `value` | `0.419` |

### Disabling Metrics

List metric names under `disabled_metrics` to suppress them from `mdlr check` output (text rows, JSON fields, and the per-symbol view):

```yaml
disabled_metrics:
  - lcom
  - duplication_pct
  - uncov_branches
```

Use the canonical metric names shown by `mdlr metrics ls` — `fan_in`, `fan_out`, `function_size`, `params`, `cyclomatic`, `cognitive`, `max_scope`, `methods_per_struct`, `lcom`, `file_loc`, `duplication_pct`, `dag_density`, `line_cov`, `uncov_branches`. Note these are the *metric* names, not the threshold keys: disable `fan_in`, not `fan_in_max`.

Behavior:

- **Output-control, not compute-control.** A disabled metric is omitted from output, but most metrics share a bundled computation pass, so disabling one rarely saves work. The exceptions are skipped outright: the copy-paste detection pass is skipped when `duplication_pct` is disabled, and coverage parsing is skipped when both `line_cov` and `uncov_branches` are disabled.
- **Composite JSON objects** (`complexity`, `struct`, `coverage`) drop only the disabled sub-fields, and are omitted entirely when all their metrics are disabled.
- **Unknown names** are reported as a warning on stderr and ignored — a typo leaves the metric enabled, so check the warning.
- `mdlr metrics ls` and `mdlr metrics get <name>` annotate disabled metrics with `(disabled)`.

## Default Thresholds

The default thresholds are based on empirical observations of healthy codebases:

### Structural Metrics

| Metric | Excellent | Good | Fair | Poor | Critical |
|--------|-----------|------|------|------|----------|
| dag_density | < 0.5 | < 1.0 | < 1.5 | < 2.0 | >= 2.0 |
| fan_in_max | < 3 | < 5 | < 10 | < 15 | >= 15 |
| fan_in_mean | < 0.5 | < 1.0 | < 2.0 | < 3.0 | >= 3.0 |
| fan_out_max | < 3 | < 5 | < 8 | < 12 | >= 12 |
| fan_out_mean | < 0.5 | < 1.0 | < 2.0 | < 3.0 | >= 3.0 |

### Complexity Metrics

| Metric | Excellent | Good | Fair | Poor | Critical |
|--------|-----------|------|------|------|----------|
| function_size | < 20 | < 50 | < 100 | < 200 | >= 200 |
| params | < 3 | < 5 | < 7 | < 10 | >= 10 |
| cyclomatic | < 5 | < 10 | < 20 | < 30 | >= 30 |
| cognitive | < 5 | < 10 | < 15 | < 25 | >= 25 |
| max_scope | < 15 | < 30 | < 50 | < 100 | >= 100 |

### File Metrics

| Metric | Excellent | Good | Fair | Poor | Critical |
|--------|-----------|------|------|------|----------|
| file_loc | < 200 | < 400 | < 600 | < 1000 | >= 1000 |
| duplication_pct | < 3 | < 5 | < 10 | < 20 | >= 20 |

### Impl Metrics

| Metric | Excellent | Good | Fair | Poor | Critical |
|--------|-----------|------|------|------|----------|
| methods_per_struct | < 5 | < 10 | < 15 | < 25 | >= 25 |
| lcom | < 2 | < 3 | < 4 | < 5 | >= 5 |

### Coverage Metrics

Emitted only when `--cov <lcov>` is passed to `mdlr check`. `line_cov` is **lower-is-worse**: each threshold is the low boundary of that bucket (e.g. value >= 90 is excellent, value < 60 is critical).

| Metric | Excellent | Good | Fair | Poor | Critical |
|--------|-----------|------|------|------|----------|
| line_cov | >= 90 | >= 80 | >= 70 | >= 60 | < 60 |
| uncov_branches | < 1 | < 3 | < 6 | < 10 | >= 10 |

## Example Configuration

```yaml
# Stricter thresholds for a mature codebase
thresholds:
  dag_density:
    excellent: 0.3
    good: 0.6
    fair: 1.0
    poor: 1.5

  fan_out_max:
    excellent: 2
    good: 4
    fair: 6
    poor: 8

# Show both value and label
display:
  mode: both
```

## Partial Configuration

You only need to specify the values you want to override. All unspecified values use defaults:

```yaml
# Only override dag_density thresholds
thresholds:
  dag_density:
    excellent: 0.3
    good: 0.7
    fair: 1.2
    poor: 1.8
```

## Output Examples

**Text output (mode: both):**
```
Structural Metrics
==================

DAG Density: 0.419 (excellent)

Fan-In:  max=4 (good), mean=0.43 (excellent)
Fan-Out: max=6 (fair), mean=0.43 (excellent)
```

**JSON output:**
```json
{
  "metrics": {
    "dag_density": {
      "value": 0.419,
      "bucket": "excellent"
    },
    "fan_in": {
      "max": { "value": 4, "bucket": "good" },
      "mean": { "value": 0.43, "bucket": "excellent" }
    },
    "fan_out": {
      "max": { "value": 6, "bucket": "fair" },
      "mean": { "value": 0.43, "bucket": "excellent" }
    }
  }
}
```
