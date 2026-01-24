# Impl Metrics

Impl metrics measure the structure and cohesion of `impl` blocks (Rust's equivalent of classes). These help identify god classes, interface pollution, and lack of cohesion.

## Metrics

### Methods per Impl

Counts the number of methods in each impl block.

| Statistic | Description |
|-----------|-------------|
| max | Most methods in any impl |
| mean | Average methods per impl |
| p90 | 90th percentile |

**Default thresholds:**

| Bucket | Threshold |
|--------|-----------|
| Excellent | < 5 methods |
| Good | < 10 methods |
| Fair | < 15 methods |
| Poor | < 25 methods |
| Critical | >= 25 methods |

**Why it matters:** Impls with many methods often indicate a "god class" that has too many responsibilities. Consider splitting into multiple focused types.

### Traits per Type

Counts how many traits each type implements.

| Statistic | Description |
|-----------|-------------|
| max | Most traits on any type |
| mean | Average traits per type |

**Default thresholds:**

| Bucket | Threshold |
|--------|-----------|
| Excellent | < 3 traits |
| Good | < 5 traits |
| Fair | < 8 traits |
| Poor | < 12 traits |
| Critical | >= 12 traits |

**Why it matters:** Types implementing many traits may have unclear responsibilities or be trying to satisfy too many interfaces. This can indicate interface pollution.

### LCOM (Lack of Cohesion of Methods)

Measures how cohesive an impl is based on shared field access between methods.

LCOM compares method pairs:
- **Cohesive pair**: Two methods that both access at least one common field
- **Incohesive pair**: Two methods that share no field access

The metric is computed as:
```
LCOM = max(0, incohesive_pairs - cohesive_pairs) / total_pairs
```

Normalized to 0-1 where:
- **0** = Perfectly cohesive (all methods work on the same data)
- **1** = Completely incohesive (no methods share data)

| Statistic | Description |
|-----------|-------------|
| max | Highest LCOM (least cohesive impl) |
| mean | Average LCOM across impls |

**Default thresholds:**

| Bucket | Threshold |
|--------|-----------|
| Excellent | < 0.2 |
| Good | < 0.4 |
| Fair | < 0.6 |
| Poor | < 0.8 |
| Critical | >= 0.8 |

**Why it matters:** High LCOM indicates methods in an impl don't work together on shared data. This suggests the impl might be doing unrelated things and should be split.

## Example Output

```
Impl Metrics
============

Methods/Impl: max=17, mean=2.1, p90=5
Traits/Type:  max=2, mean=1.1
LCOM:         max=1.00, mean=0.18

Largest Impls:
  impl CacheStore (17 methods)
  impl SemanticTags (7 methods)

Types with Many Traits:
  Config (3 traits)

Least Cohesive Impls (high LCOM):
  impl ComplexityMetrics (LCOM=1.00)
  impl CacheStore (LCOM=0.84)
```

## Interpretation

- **Large impls (many methods)**: Consider the Single Responsibility Principle. Can this be split into multiple focused types?
- **Many traits per type**: Is this type trying to do too much? Could some traits be combined or the type split?
- **High LCOM**: Methods aren't working on shared data. Either:
  - The impl should be split into cohesive groups
  - Methods are stateless utilities (which is fine)
  - Field tracking may be incomplete (check if methods access fields through nested calls)

## Configuration

```yaml
thresholds:
  methods_per_impl:
    excellent: 5
    good: 10
    fair: 15
    poor: 25

  traits_per_type:
    excellent: 3
    good: 5
    fair: 8
    poor: 12

  lcom:
    excellent: 0.2
    good: 0.4
    fair: 0.6
    poor: 0.8
```

## Field Access Tracking

LCOM requires tracking which fields each method reads and writes. The extractor tracks:

- `self.field` read access
- `self.field = value` write access

Limitations:
- Field access through method calls is not tracked
- Field access in closures may not be attributed correctly
