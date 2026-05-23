# Interpreting Results

## Healthy Patterns

Different types of modules naturally have different metric profiles:

| Module Type | Fan-In | Fan-Out | Example |
|-------------|--------|---------|---------|
| Utility | High | Low | String helpers, validators |
| Orchestration | Low | High | Main function, controllers |
| Leaf | Low | Low | Specialized algorithms |
| Hub | High | High | Core domain objects (warning) |

### Utility Modules
- **Expected**: High fan-in, low fan-out
- **Why**: Widely used, few dependencies
- **Example**: A `format_date` function used everywhere but depending on nothing

### Orchestration Modules
- **Expected**: Low fan-in, high fan-out
- **Why**: Coordinate many things, few depend on them
- **Example**: `main()` function wiring up the application

### Leaf Modules
- **Expected**: Low fan-in, low fan-out
- **Why**: Specialized, focused functionality
- **Example**: A specific algorithm implementation

## Warning Signs

| Pattern | Symptom | Concern |
|---------|---------|---------|
| High fan-out + high fan-in | Hub unit | Change ripples everywhere; hard to modify safely |
| Very high DAG density | Everything connected | Tight coupling; hard to extract or modify independently |
| Many units with 0 fan-in | Dead code or entry points | If not entry points, may be unused code |
| Single unit with extreme fan-out | God function/class | Doing too much; violates single responsibility |

### Hub Units (High Fan-In + High Fan-Out)

These are the most problematic. They:
- Are depended upon by many (can't change interface easily)
- Depend on many (affected by many changes)
- Create coupling between otherwise unrelated modules

**Solution**: Break into smaller units with clearer responsibilities.

### Very High DAG Density

When density >> 2.0, the codebase is highly interconnected. This makes it:
- Hard to understand any single part in isolation
- Risky to change (unexpected ripple effects)
- Difficult to test (many dependencies to mock)

**Solution**: Identify and break circular dependencies, introduce interfaces/abstractions.

## Bucket Labels

mdlr displays bucket labels alongside metric values to help quickly assess code health. The five buckets are:

| Bucket | Meaning |
|--------|---------|
| excellent | Well within healthy range |
| good | Healthy, no action needed |
| fair | Acceptable, consider monitoring |
| poor | Action recommended |
| critical | Requires attention |

Labels and thresholds can be customized via [configuration](../reference/config.md).

## Example Analysis

```
Analysis for session 'my-project'

Graph: 87 units, 36 edges

Structural Metrics
==================

DAG Density: 0.419 (excellent)

Fan-In:  max=4 (good), mean=0.43 (excellent)
Fan-Out: max=6 (fair), mean=0.43 (excellent)

Top Fan-Out:
  extract_from_node (6)
  main (4)
  build_graph (3)

Top Fan-In:
  get_node_name (4)
  node_span (4)
  compute (3)
```

**Reading this output:**

1. **DAG Density 0.419 (excellent)**: Well below 1.0, indicating a loosely coupled codebase. The "excellent" bucket confirms this is healthy.

2. **Fan-Out max=6 (fair)**: `extract_from_node` has the highest fan-out. The "fair" label suggests it's acceptable but worth monitoring. This is expected - it's an orchestration function that dispatches to various extractors.

3. **Fan-In max=4 (good)**: `get_node_name` and `node_span` are the most reused utilities. "Good" indicates healthy reuse levels. They should be stable and well-tested.

4. **Mean values ~0.43 (excellent)**: Low average connectivity suggests most units are focused and independent.

## Tracking Over Time

Metrics are most valuable when tracked over time:

```bash
# Weekly snapshot
mdlr check -A --format json >> metrics-history.jsonl
```

Watch for:
- DAG density creeping upward (increasing coupling)
- New hub units appearing (centralization)
- Fan-out max increasing (growing god functions)
