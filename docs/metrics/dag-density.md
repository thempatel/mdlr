# DAG Density

## Definition

```
dag_density = edges / (nodes - 1)
```

DAG density measures how connected the dependency graph is relative to a minimal tree structure (which would have exactly `nodes - 1` edges).

## Interpretation

| Value | Interpretation |
|-------|----------------|
| 1.0 | Linear chain - each unit depends on exactly one other (minimal connectivity) |
| < 1.0 | Forest - disconnected components exist |
| > 1.0 | More interconnected than a tree - units have multiple dependencies |
| >> 1.0 | Highly interconnected - potential coupling concerns |

## What It Tells You

A density significantly above 1.0 suggests the codebase has many cross-cutting dependencies. This isn't inherently bad, but very high values may indicate tight coupling that makes the code harder to modify independently.

## Examples

### Density = 1.0 (Linear Chain)
```
A → B → C → D
```
4 nodes, 3 edges: `3 / (4-1) = 1.0`

### Density < 1.0 (Forest)
```
A → B    C → D
```
4 nodes, 2 edges: `2 / (4-1) = 0.67`

### Density > 1.0 (Diamond)
```
    A
   ↙ ↘
  B   C
   ↘ ↙
    D
```
4 nodes, 4 edges: `4 / (4-1) = 1.33`

## Guidelines

- **< 0.5**: Very loosely coupled, possibly disconnected modules
- **0.5 - 1.0**: Loosely coupled, tree-like structure
- **1.0 - 2.0**: Moderate coupling, typical for cohesive modules
- **> 2.0**: Tightly coupled, consider refactoring to reduce dependencies

## When Density Is Very High

When density climbs well above 2.0, the codebase is highly interconnected, which compounds in a few painful ways:

- **Hard to understand any single part in isolation** — every unit drags context from many others.
- **Risky to change** — edits ripple through dependents in non-obvious ways.
- **Difficult to test** — many dependencies to set up or mock for any given unit.

The usual recourse is to identify and break circular dependencies and introduce interfaces or abstractions that let groups of units evolve independently. Pair this with fan-in / fan-out analysis to find the specific hubs driving the density up.
