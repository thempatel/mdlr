# Complexity Metrics

Complexity metrics measure how complex individual functions are, helping identify functions that may be doing too much.

## Metrics

### Function Size

Lines of code per function, computed from the span (start line to end line).

`function_size` is a **two-sided metric**: both extremes are bad. Too big is hard to understand and test; too small (a 1-2 line pass-through) adds indirection without abstraction. A value is evaluated against two threshold sets — `high` (higher-is-worse) and `low` (lower-is-worse) — and gets the worse of the two buckets.

The low side applies **only to functions with exactly one visible caller** (`fan_in == 1`) — the case where "inline it into the caller" is well-defined. Functions with unknown callers (`fan_in == 0`: trait dispatch, public API, entry points) or multiple callers (shared helpers, accessors) are exempt from the low side and evaluated against the high side only.

| Statistic | Description |
|-----------|-------------|
| max | Largest function in lines |
| mean | Average function size |
| p90 | 90th percentile size |

The aggregate statistics describe the high tail; tiny flagged functions surface as individual rows.

**Default thresholds (high side, always applies):**

| Bucket | Threshold |
|--------|-----------|
| Excellent | < 20 lines |
| Good | < 50 lines |
| Fair | < 100 lines |
| Poor | < 200 lines |
| Critical | >= 200 lines |

**Default thresholds (low side, only when `fan_in == 1`):**

| Bucket | Threshold |
|--------|-----------|
| Excellent | >= 5 lines |
| Good | 4 lines |
| Fair | 3 lines |
| Poor | <= 2 lines |

Critical is unreachable on the low side by default, so a 1-liner never outranks a god function in worst-first ordering.

### Parameter Count

Number of parameters per function. Self/&self/&mut self parameters are not counted.

| Statistic | Description |
|-----------|-------------|
| max | Most parameters on any function |
| mean | Average parameter count |

**Default thresholds:**

| Bucket | Threshold |
|--------|-----------|
| Excellent | < 3 params |
| Good | < 5 params |
| Fair | < 7 params |
| Poor | < 10 params |
| Critical | >= 10 params |

### Cyclomatic Complexity

Measures the number of independent paths through a function. Higher values indicate more complex control flow.

Counts:
- `if` expressions (+1 each)
- `match` arms (+1 per arm beyond the first)
- `while`, `for`, `loop` expressions (+1 each)
- `&&` and `||` operators (+1 each)

| Statistic | Description |
|-----------|-------------|
| max | Highest complexity in any function |
| mean | Average complexity |
| p90 | 90th percentile complexity |

**Default thresholds:**

| Bucket | Threshold |
|--------|-----------|
| Excellent | < 5 |
| Good | < 10 |
| Fair | < 20 |
| Poor | < 30 |
| Critical | >= 30 |

### Max Scope Lines

Measures the largest single scope block within each function. While `function_size` measures the overall length, `max_scope` catches functions where a single block (if body, match arm, loop body, closure) dominates — suggesting that block should be extracted into its own function.

| Statistic | Description |
|-----------|-------------|
| max | Largest scope block across all functions |
| mean | Average max scope size |
| p90 | 90th percentile max scope size |

**Default thresholds:**

| Bucket | Threshold |
|--------|-----------|
| Excellent | < 15 lines |
| Good | < 30 lines |
| Fair | < 50 lines |
| Poor | < 100 lines |
| Critical | >= 100 lines |

## Example Output

```
Complexity Metrics
==================

Function Size: max=294 lines, mean=17.3, p90=28
Parameters:    max=6, mean=0.9
Cyclomatic:    max=22, mean=2.5, p90=5

Most Complex Functions:
  handle_check (cc=22, lines=294, params=3)
  handle_tag (cc=17, lines=108, params=6)
  count_branches_recursive (cc=14, lines=41, params=2)

Largest Functions:
  handle_check (294 lines)
  handle_tag (108 lines)
```

## Interpretation

- **High function size**: Consider breaking into smaller, focused functions
- **Low function size** (flagged only when `fan_in == 1`): A single-caller pass-through — consider inlining it into its caller. Never inline trait-required methods, public API accessors, or shared helpers; those are already exempt from the flag
- **Many parameters**: Consider using a struct/builder pattern
- **High cyclomatic complexity**: Consider extracting conditional logic into separate functions
- **High max scope**: Extract the oversized block into a separate function

## Configuration

```yaml
thresholds:
  # Two-sided: low and high blocks. The old flat form
  # (excellent/good/fair/poor) is still accepted and
  # configures the high side.
  function_size:
    low:            # lower-is-worse; only applies when fan_in == 1
      excellent: 5  # >= 5 lines is fine
      good: 4
      fair: 3
      poor: 1       # <= 2 lines is poor; critical unreachable
    high:           # higher-is-worse
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

  max_scope:
    excellent: 15
    good: 30
    fair: 50
    poor: 100
```
