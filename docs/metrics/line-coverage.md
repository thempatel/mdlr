# Line Coverage

Per-function test coverage percentage, derived from an LCOV file passed via `mdlr check --cov`.

## How It's Computed

1. The LCOV file is parsed for `SF:`, `DA:line,hits`, and `BRDA:line,...,taken` records.
2. For each function/method `Unit`, mdlr collects every `DA` record whose line falls inside the unit's span, **excluding** lines that belong to a nested unit (a closure or method declared inside this function).
3. `line_cov = (lines with hits > 0) / (total attributed lines) * 100`, rounded down.

A `Unit` with **zero** attributed `DA` records reports `0%` and is counted toward the "no data" warning.

## Innermost-Containing-Unit Rule

If a closure spans lines 20-30 inside a function spanning lines 1-50, lines 20-30 attribute to the closure, not the outer function. This means a function with two nested closures is measured only against its own body lines, and the closures get their own rows.

## Sort Direction

`line_cov` is the first metric in mdlr that uses `SortDirection::Asc` — **lower values are worse**. The distribution is sorted worst-first (smallest %), so `-k 3` shows the three least-covered functions.

## Effect on Ranking

Passing `--cov` also changes the order of the text worklist, not just the rows in it. Within one metric and one severity bucket, the least-covered unit is listed first: among functions the same metric calls `critical`, an untested one is a worse use of the next editing pass than an equally complex one the tests already exercise.

The tie-break never reorders across metrics. Severity buckets are coarse — hundreds of rows can share `critical` — so ranking a whole bucket by coverage would let a trivial untested unit outrank a genuinely alarming one under a different metric. Without `--cov` the order is exactly what it was.

`--format json` emits each metric's full distribution in its own order, so its ordering is unchanged either way.

## Default Thresholds

| Bucket    | Value           |
|-----------|-----------------|
| Excellent | `>= 90%`        |
| Good      | `80% – 89%`     |
| Fair      | `70% – 79%`     |
| Poor      | `60% – 69%`     |
| Critical  | `< 60%`         |

Override these per-project via `.mdlr/config.yaml`:

```yaml
thresholds:
  line_cov:
    excellent: 95.0
    good:      85.0
    fair:      75.0
    poor:      65.0
```

For an ascending metric, each field names the **low boundary** of that bucket. A value at-or-above the field is in that bucket or better.

## Interpreting Results

| Symptom                                          | Likely cause                                                                                 |
|--------------------------------------------------|----------------------------------------------------------------------------------------------|
| Warning `lcov references N file(s) but none match any analyzed source` | LCOV `SF:` paths don't match any source file mdlr sees. Common for TS/JS projects where `nyc` ran against pre-built `dist/*.js` without sourcemaps — the LCOV references `.js` but the graph holds `.ts`. Fix by running coverage on the source (e.g. `vitest --coverage`, `jest` with `ts-jest`) or applying sourcemaps before LCOV emission. Path-rooting mismatches (`--root` vs. where the coverage tool ran) trigger the same warning. |
| Whole project reports `line_cov: 0` (no warning)  | Stale lcov, or `--cov` points at a file from an older run. Re-run your coverage tool, re-pass the file. |
| One function `line_cov: 0`, rest are healthy     | Net-new function added without a test, or function body is unreachable.                       |
| `line_cov: 100`, but you know it's barely tested | DA records are sparse — lcov may only cover one happy path. Check `uncov_branches`.           |

## Caveats

- LCOV's `SF:` paths must resolve to the same canonical filesystem path as mdlr's `Unit.file`. Both sides are canonicalized; relative paths are resolved against the project root.
- Coverage percentage is integer (0-100), not floating-point. A function with 7 lines hit out of 9 reports `77`, not `77.78`.

## Related

- [Uncovered Branches](uncov-branches.md) — paired metric for path coverage
- [CLI Reference: `--cov`](../reference/cli.md#check)
