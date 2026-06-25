# Fan-Out

## Definition

Fan-out is the number of outgoing edges from a unit - how many other units it depends on or calls.

## Reported Values

| Metric | Description |
|--------|-------------|
| max | Highest fan-out of any single unit |
| mean | Average fan-out across all units |
| distribution | List of units sorted by fan-out (top 10 shown) |

## Interpretation

**High fan-out indicates:**
- The unit has many dependencies
- Changes to dependencies may require updating this unit
- The unit may be doing too much (potential god function/class)
- Consider breaking it into smaller, focused units

**Low fan-out indicates:**
- The unit is self-contained or a leaf node
- Few external dependencies
- Easier to understand and test in isolation

## Example

A function `extract_from_node` with fan-out of 6 calls 6 other functions:

```
extract_from_node ──→ extract_function
                 ├──→ extract_struct
                 ├──→ extract_trait
                 ├──→ extract_impl
                 ├──→ get_node_name
                 └──→ node_span
```

If any of those 6 functions change their interface, `extract_from_node` may need updates.

## Guidelines

| Fan-Out | Interpretation |
|---------|----------------|
| 0-2 | Low complexity, focused unit |
| 3-5 | Moderate complexity, typical |
| 6-10 | High complexity, may benefit from decomposition |
| > 10 | Very high complexity - strong candidate for refactoring |

## Delegator suppression

A high fan-out is only worth flagging when it comes with real internal
complexity. A unit that calls many others but barely branches is a
**Delegator** — it just forwards work to its callees, which is usually good
design, not a refactoring target.

`mdlr check` detects Delegators and omits their fan-out from the global /
top-k listing. A unit is a Delegator when **both** its `cyclomatic` and its
`cognitive` complexity sit below their `fair` thresholds. So fan-out surfaces
in the listing only for units that call a lot **and** branch/nest a lot.

This filtering applies to the ranked listing only. The value is always
available for a specific unit via `mdlr check <symbol>`, regardless of
Delegator status. The detection reads the computed complexity values
directly, so it still works when `cyclomatic` or `cognitive` is in
`disabled_metrics`.

## What To Do

**High fan-out units should be examined for:**
- Single responsibility violations
- Orchestration logic that could be simplified
- Opportunities to extract helper functions
- Facade patterns hiding complexity

**Acceptable high fan-out:**
- Main/entry points that wire things together
- Facade classes intentionally aggregating functionality
- Test setup functions

For interpreting fan-out alongside fan-in (utility / orchestration / leaf / hub), see [Reading Fan-In Alongside Fan-Out](fan-in.md#reading-fan-in-alongside-fan-out).
