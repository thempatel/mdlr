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

### Labels

Customize the bucket labels displayed with metric values:

```yaml
labels:
  excellent: "excellent"
  good: "good"
  fair: "fair"
  poor: "poor"
  critical: "critical"
```

### Thresholds

Configure thresholds for each metric. Values below a threshold receive that bucket label. Values at or above the `poor` threshold are labeled `critical`.

```yaml
thresholds:
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

## Default Thresholds

The default thresholds are based on empirical observations of healthy codebases:

| Metric | Excellent | Good | Fair | Poor | Critical |
|--------|-----------|------|------|------|----------|
| dag_density | < 0.5 | < 1.0 | < 1.5 | < 2.0 | >= 2.0 |
| fan_in_max | < 3 | < 5 | < 10 | < 15 | >= 15 |
| fan_in_mean | < 0.5 | < 1.0 | < 2.0 | < 3.0 | >= 3.0 |
| fan_out_max | < 3 | < 5 | < 8 | < 12 | >= 12 |
| fan_out_mean | < 0.5 | < 1.0 | < 2.0 | < 3.0 | >= 3.0 |

## Example Configuration

```yaml
# Customize labels for your team
labels:
  excellent: "ship it"
  good: "acceptable"
  fair: "needs work"
  poor: "refactor"
  critical: "urgent"

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
