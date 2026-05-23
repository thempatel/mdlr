# Fan-In

## Definition

Fan-in is the number of incoming edges to a unit - how many other units depend on or call it.

## Reported Values

| Metric | Description |
|--------|-------------|
| max | Highest fan-in of any single unit |
| mean | Average fan-in across all units |
| distribution | List of units sorted by fan-in (top 10 shown) |

## Interpretation

**High fan-in indicates:**
- The unit is widely used (high reuse)
- Changes to this unit affect many dependents
- This is a critical/core piece of the codebase
- Extra care needed when modifying - consider stability

**Low fan-in indicates:**
- The unit has few or no dependents
- May be a leaf node, entry point, or potentially dead code
- Changes have limited blast radius

## Example

A utility function `get_node_name` with fan-in of 4 is called from 4 different places. Changing its signature requires updating all 4 call sites.

```
extract_function ──→ get_node_name ←── extract_struct
                           ↑
extract_trait ─────────────┘
                           ↑
extract_impl ──────────────┘
```

## Guidelines

| Fan-In | Interpretation |
|--------|----------------|
| 0 | Entry point, dead code, or test-only |
| 1-3 | Normal, limited usage |
| 4-10 | Moderate reuse, somewhat critical |
| > 10 | High reuse, very critical - treat as stable API |

## What To Do

**High fan-in units should:**
- Have stable interfaces (avoid breaking changes)
- Be well-tested
- Be documented
- Have clear contracts

**Zero fan-in units might be:**
- Entry points (main, handlers) - expected
- Dead code - consider removing
- Test utilities - expected

## Reading Fan-In Alongside Fan-Out

Fan-in and fan-out only mean something in combination. The four corners of the matrix correspond to recognisable module shapes:

| Module Type | Fan-In | Fan-Out | Example |
|-------------|--------|---------|---------|
| Utility | High | Low | String helpers, validators — widely used, depend on nothing |
| Orchestration | Low | High | `main()`, controllers — coordinate many things, few depend on them |
| Leaf | Low | Low | Specialized algorithms — focused, self-contained |
| Hub | High | High | Core domain objects — **warning sign** |

**Hubs (high fan-in + high fan-out) are the most problematic combination:**

- Many units depend on them, so their interface is hard to change without ripple effects.
- They depend on many units, so they're affected by many changes.
- They create coupling between otherwise unrelated parts of the codebase.

When you spot a hub, the fix is usually to split it into smaller units with clearer responsibilities. Pull out the cohesive groups; let each piece be either widely used *or* doing a lot, not both.
