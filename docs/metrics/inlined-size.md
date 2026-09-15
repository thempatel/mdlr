# Inlined Size

## Definition

`inlined_size` is the number of lines a unit would have if every helper only it calls were folded back into it.

A callee is absorbed when all of the following hold:

| Condition | Why |
|-----------|-----|
| The parent has a `Calls` edge to it | It is actually used by this unit |
| Exactly one distinct unit references it | Nothing else can reach it, so it belongs to this parent alone |
| It lives in the same file as the parent | A helper moved to its own module is a real boundary |
| It is a function or method | Structs and modules have no size |

Absorption is transitive: a helper's own exclusive helpers count too. Because an absorbed unit has exactly one referrer, each one belongs to a single parent, so nothing is double-counted.

## Why it exists

`function_size` rewards splitting a unit up, whether or not the split made anything simpler. Moving 280 lines out of a 300-line function into ten helpers nothing else calls leaves eleven units that all read as healthy, without deleting a single line:

```
before                          after
run_pipeline        300 lines   run_pipeline         30 lines   good
                                run_pipeline_part_1  28 lines   good
                                ... eight more ...
                                run_pipeline_part_10 28 lines   good
```

Every `function_size` row improved. The codebase got one line longer. `inlined_size` reports `run_pipeline` at 310 either way, because the lines pushed into those helpers stay on the parent's total.

## Reported Values

| Metric | Description |
|--------|-------------|
| inlined_size | Own lines plus every absorbed exclusive helper, transitively |
| min_exclusive_fanout | The gate width in force for the run, echoed in JSON output |

## The gate

The value alone is not reportable. A chain of steps that call each other one at a time — `step_01` calls `step_02` calls `step_03` — is also a single-caller tree, and absorbing it reports the first step as the size of the whole pipeline. That is a false positive on ordinary code.

A row reaches the global listing only when **both** hold:

| Condition | Default | Rejects |
|-----------|---------|---------|
| The unit absorbs at least this many *direct* helpers nothing else calls (`inlined.min_exclusive_fanout`) | 3 | Chains, which are one helper deep at each level, and ordinary one-or-two-helper decompositions |
| The unit's own `function_size` is below its `fair` threshold | 100 lines | Units that are simply large — `function_size` already reports those, in a stronger bucket |

Dispersal is wide: one parent, many leaves. A chain is narrow: one leaf per level. Width is what separates them, and no amount of tuning the value can.

The second condition is what keeps the metric from restating `function_size`. On a 346-line function that absorbs 17 lines, `inlined_size` would say `poor` where `function_size` already said `critical` — strictly less information.

`mdlr check <symbol>` ignores the gate and always shows the value.

## Interpretation

**A high value means:** the split was cosmetic. The work still belongs to one unit; only the line count moved.

Fixes, in rough order of preference:

- **Delete code.** If the helpers exist to make a large function look small, the underlying function is still too large.
- **Give the helpers other callers.** A helper used from two places is real reuse and stops being absorbed.
- **Move them to their own module.** A helper in another file is a real boundary and stops being absorbed.

Splitting further does not help — it raises the width and keeps the total.

## Guidelines

| inlined_size | Interpretation |
|--------------|----------------|
| < 20 | Fine |
| 20-50 | Typical |
| 50-100 | Worth a look |
| 100-200 | The unit owns more than it appears to |
| > 200 | A large unit wearing a small one's line count |

These are deliberately the same bands as `function_size`'s high side: the value answers "what would `function_size` say if this were put back together", so the two agree on what counts as large.

## Example

```
$ mdlr check mdlr::symbol_commands::handle_ls
metric          symbol                            value  bucket
function_size   mdlr::symbol_commands::handle_ls  26     good
inlined_size    mdlr::symbol_commands::handle_ls  112    poor
```

26 lines of its own, plus four helpers nothing else calls: `collect_units` (29), `print_ls_text` (26), `print_ls_json` (18) and `parse_unit_kind` (13). 26 + 29 + 26 + 18 + 13 = 112 lines whose only entry point is this command.

## Configuration

```yaml
thresholds:
  inlined_size:
    excellent: 20
    good: 50
    fair: 100
    poor: 200
```

The bands above set the bucket. The gate width, which decides whether the row is listed at all, is separate:

```yaml
inlined:
  min_exclusive_fanout: 3
```

Disable with:

```yaml
disabled_metrics:
  - inlined_size
```

## Related

- [Function Size](complexity.md#function-size) — the metric this one backstops
- [Fan-Out](fan-out.md) — outgoing edges, not restricted to exclusive helpers
- [Overview](overview.md)
