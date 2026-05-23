# Impl Metrics

Impl metrics measure the size and cohesion of structs and their `impl` blocks (Rust's equivalent of classes). These help identify god classes and lack of cohesion.

## Metrics

### Methods per Struct

Counts the number of methods defined on each struct (aggregated across its `impl` blocks).

| Statistic | Description |
|-----------|-------------|
| max | Most methods on any struct |
| mean | Average methods per struct |
| p90 | 90th percentile |

**Default thresholds:**

| Bucket | Threshold |
|--------|-----------|
| Excellent | < 5 methods |
| Good | < 10 methods |
| Fair | < 15 methods |
| Poor | < 25 methods |
| Critical | >= 25 methods |

**Why it matters:** Structs with many methods often indicate a "god class" that has too many responsibilities. Consider splitting into multiple focused types.

### LCOM4 (Lack of Cohesion of Methods)

Measures how cohesive a struct is by counting connected components in a method graph.

LCOM4 builds an undirected graph where:
- **Nodes** are methods of the struct
- **Edges** connect two methods if they share access to a common field OR one calls the other

LCOM4 = the number of connected components in this graph.

- **1** = All methods are related (cohesive)
- **2+** = The struct has unrelated groups of methods and could be split

| Statistic | Description |
|-----------|-------------|
| max | Highest LCOM4 (least cohesive struct) |
| mean | Average LCOM4 across structs |

**Default thresholds:**

| Bucket | Threshold |
|--------|-----------|
| Excellent | < 2 |
| Good | < 3 |
| Fair | < 4 |
| Poor | < 5 |
| Critical | >= 5 |

**Why it matters:** LCOM4 >= 2 means the struct contains unrelated groups of methods that don't share state or call each other. Each connected component could potentially be its own struct.

## Example Output

```
$ mdlr check --pretty
metric              symbol                                  value  bucket
methods_per_struct  mdlr::cache::store::CacheStore          17     fair
methods_per_struct  mdlr_core::semantic::SemanticTags       7      good
lcom                mdlr_metrics::complexity::ComplexityMetrics  3   fair
lcom                mdlr::cache::store::CacheStore          2      good
```

## Interpretation

- **Large structs (many methods)**: Consider the Single Responsibility Principle. Can this be split into multiple focused types?
- **LCOM4 >= 2**: The struct has disconnected groups of methods. Either:
  - The struct should be split into cohesive groups (one per connected component)
  - Methods are stateless utilities (which is fine)
  - Field tracking may be incomplete (check if methods access fields through nested calls)

## Configuration

```yaml
thresholds:
  methods_per_struct:
    excellent: 5
    good: 10
    fair: 15
    poor: 25

  lcom:
    excellent: 2
    good: 3
    fair: 4
    poor: 5
```

## Method Connectivity Tracking

LCOM4 connects methods that share field access or call each other. The extractor tracks:

- `self.field` read access
- `self.field = value` write access
- Method-to-method calls within the same struct

Limitations:
- Field access through nested method calls is not tracked
- Field access in closures may not be attributed correctly
