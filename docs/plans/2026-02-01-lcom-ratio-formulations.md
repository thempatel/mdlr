---
title: LCOM Ratio Formulations for False Positive Reduction
date: 2026-02-01
status: draft
---

# LCOM Ratio Formulations for False Positive Reduction

## Background

LCOM (Lack of Cohesion of Methods) measures whether methods in a struct share field access. The current implementation produces false positives for common patterns:

- **Constructors**: Write all fields but don't read them
- **Builders**: Chain of `with_*` methods each writing one field, `build` reading all
- **Data structs**: Constructor sets fields, fields accessed directly (public) or via simple getters
- **Factories**: Multiple `from_*` methods that don't share field access with each other

### Current Algorithm (Normalized LCOM1)

From `crates/mdlr-metrics/src/struct_metrics.rs`:

```rust
// Count pairs of methods that share fields vs don't share
for i in 0..methods.len() {
    for j in (i + 1)..methods.len() {
        let fields_i: HashSet<_> = methods[i].reads.iter()
            .chain(methods[i].writes.iter())
            .collect();
        let fields_j: HashSet<_> = methods[j].reads.iter()
            .chain(methods[j].writes.iter())
            .collect();

        if fields_i.intersection(&fields_j).next().is_some() {
            shares_field += 1;
        } else {
            no_shared_field += 1;
        }
    }
}

// LCOM = max(0, P - Q) / total_pairs
```

**Formula**: `LCOM = max(0, P - Q) / total_pairs` where P = non-sharing pairs, Q = sharing pairs.

**Range**: 0.0 (fully cohesive) to 1.0 (fully incohesive)

### Why False Positives Occur

Consider a builder:
```rust
impl Builder {
    fn new() -> Self { Self { name: None, value: None } }  // writes: [name, value]
    fn with_name(mut self, n: String) -> Self { self.name = Some(n); self }  // writes: [name]
    fn with_value(mut self, v: i32) -> Self { self.value = Some(v); self }  // writes: [value]
    fn build(self) -> Thing { Thing { name: self.name.unwrap(), value: self.value.unwrap() } }  // reads: [name, value]
}
```

Method pairs and field overlap:
| Pair | Fields A | Fields B | Overlap? |
|------|----------|----------|----------|
| new, with_name | {name, value} | {name} | Yes |
| new, with_value | {name, value} | {value} | Yes |
| new, build | {name, value} | {name, value} | Yes |
| with_name, with_value | {name} | {value} | **No** |
| with_name, build | {name} | {name, value} | Yes |
| with_value, build | {value} | {name, value} | Yes |

P = 1, Q = 5, LCOM = max(0, 1-5)/6 = 0. This case works.

But consider when `new` is a static factory not tracked as accessing fields:
| Pair | Overlap? |
|------|----------|
| with_name, with_value | No |
| with_name, build | Yes |
| with_value, build | Yes |

P = 1, Q = 2, LCOM = 0. Still works.

The problem occurs when field tracking is incomplete or when methods genuinely operate on disjoint field subsets by design (service patterns).

## Goal

Find a ratio-based LCOM formulation that:

1. Naturally scores well for constructors, builders, and data structs
2. Still identifies genuinely incohesive structs (unrelated methods grouped together)
3. Produces interpretable values (0 = good, 1 = bad, or vice versa)
4. Can be computed from existing field read/write data

## Options

### Option A: Field Coverage Ratio

**Formula**:
```
shared_fields = count of fields accessed by ≥2 methods
total_fields = count of distinct fields accessed by any method
cohesion = shared_fields / total_fields
LCOM = 1 - cohesion
```

**Intuition**: What fraction of fields are "shared concerns" vs "isolated concerns"?

**Example (Builder)**:
- `name` accessed by: new, with_name, build (3 methods) → shared
- `value` accessed by: new, with_value, build (3 methods) → shared
- shared_fields = 2, total_fields = 2
- cohesion = 1.0, LCOM = 0.0

**Example (Incohesive)**:
```rust
impl Mixed {
    fn get_a(&self) -> i32 { self.a }  // reads: [a]
    fn get_b(&self) -> i32 { self.b }  // reads: [b]
    fn get_c(&self) -> i32 { self.c }  // reads: [c]
}
```
- Each field accessed by exactly 1 method
- shared_fields = 0, total_fields = 3
- cohesion = 0.0, LCOM = 1.0

**Pros**:
- Simple to compute and understand
- Naturally handles constructors (they touch many fields)
- Field-centric rather than method-pair-centric

**Cons**:
- Doesn't account for how many methods share each field
- A single constructor touching all fields makes everything "shared"

---

### Option B: Method-Field Density

**Formula**:
```
actual_accesses = count of (method, field) pairs where method accesses field
possible_accesses = method_count × field_count
density = actual_accesses / possible_accesses
LCOM = 1 - density
```

**Intuition**: How "dense" is the method-field access matrix?

**Example (Builder with 4 methods, 2 fields)**:
| | name | value |
|---|---|---|
| new | ✓ | ✓ |
| with_name | ✓ | |
| with_value | | ✓ |
| build | ✓ | ✓ |

- actual_accesses = 6
- possible_accesses = 4 × 2 = 8
- density = 0.75, LCOM = 0.25

**Example (Incohesive with 3 methods, 3 fields)**:
| | a | b | c |
|---|---|---|---|
| get_a | ✓ | | |
| get_b | | ✓ | |
| get_c | | | ✓ |

- actual_accesses = 3
- possible_accesses = 9
- density = 0.33, LCOM = 0.67

**Pros**:
- Accounts for breadth of access
- More granular than field coverage

**Cons**:
- Penalizes focused methods (a method touching 1 of 10 fields is "bad")
- May favor god methods that touch everything

---

### Option C: LCOM4 (Connected Components)

**Formula**:
```
Build undirected graph:
  - Nodes = methods
  - Edge between methods if they share at least one field
LCOM4 = connected_components - 1
```

**Intuition**: How many "clusters" of related methods exist? 0 = all connected = cohesive.

**Example (Builder)**:
- new connects to: with_name (name), with_value (value), build (name, value)
- with_name connects to: build (name)
- with_value connects to: build (value)
- All reachable from each other → 1 component
- LCOM4 = 0

**Example (Incohesive)**:
- get_a, get_b, get_c share no fields
- 3 components
- LCOM4 = 2

**Normalization** (optional):
```
LCOM4_normalized = (components - 1) / (methods - 1)
```

**Pros**:
- Standard metric with academic backing
- Naturally handles transitive cohesion
- A constructor touching all fields connects everything

**Cons**:
- Different scale than current LCOM (0 to methods-1, not 0 to 1)
- A single "bridge" method can mask structural issues
- Requires graph traversal

---

### Option D: Weighted Field Sharing

**Formula**:
```
For each field f:
  access_count[f] = number of methods accessing f

sharing_score = Σ (access_count[f])² for all fields
max_possible = methods² × fields  (if every method touched every field)
cohesion = sharing_score / max_possible
LCOM = 1 - cohesion
```

**Intuition**: Fields accessed by many methods contribute quadratically, rewarding shared concerns.

**Example (Builder with 4 methods, 2 fields)**:
- name accessed by 3 methods → 9
- value accessed by 3 methods → 9
- sharing_score = 18
- max_possible = 16 × 2 = 32
- cohesion = 0.56, LCOM = 0.44

**Example (Incohesive with 3 methods, 3 fields)**:
- Each field accessed by 1 method → 1 each
- sharing_score = 3
- max_possible = 9 × 3 = 27
- cohesion = 0.11, LCOM = 0.89

**Pros**:
- Rewards concentrated field access
- Smooth gradient between cohesive and incohesive

**Cons**:
- Quadratic weighting may be too aggressive
- Less intuitive than other formulas

---

### Option E: Pairwise Field Union

**Formula**:
```
For each pair of methods (i, j):
  fields_i = reads[i] ∪ writes[i]
  fields_j = reads[j] ∪ writes[j]
  shared[i,j] = |fields_i ∩ fields_j|
  union[i,j] = |fields_i ∪ fields_j|

total_shared = Σ shared[i,j] for all pairs
total_union = Σ union[i,j] for all pairs
cohesion = total_shared / total_union  (Jaccard-like)
LCOM = 1 - cohesion
```

**Intuition**: Across all method pairs, what fraction of their combined field footprint is shared?

**Example (Builder)**:
| Pair | Shared | Union | Ratio |
|------|--------|-------|-------|
| new, with_name | 1 | 2 | 0.5 |
| new, with_value | 1 | 2 | 0.5 |
| new, build | 2 | 2 | 1.0 |
| with_name, with_value | 0 | 2 | 0.0 |
| with_name, build | 1 | 2 | 0.5 |
| with_value, build | 1 | 2 | 0.5 |

- total_shared = 6, total_union = 12
- cohesion = 0.5, LCOM = 0.5

**Example (Incohesive)**:
| Pair | Shared | Union | Ratio |
|------|--------|-------|-------|
| get_a, get_b | 0 | 2 | 0.0 |
| get_a, get_c | 0 | 2 | 0.0 |
| get_b, get_c | 0 | 2 | 0.0 |

- total_shared = 0, total_union = 6
- cohesion = 0.0, LCOM = 1.0

**Pros**:
- Preserves pairwise comparison structure
- Accounts for method "scope" (narrow vs broad)
- Bounded 0-1

**Cons**:
- More complex calculation
- May still penalize legitimately disjoint setters

---

### Option F: Role-Aware LCOM

**Formula**:
```
Classify methods:
  - Constructors: static, named new/from_*/default, or writes-only
  - Accessors: reads single field, no writes
  - Mutators: writes fields
  - Workers: mixed read/write, business logic

Only compute LCOM over "worker" methods.
Report separately: "LCOM (workers only): X, skipped N constructors, M accessors"
```

**Intuition**: Constructors and simple accessors are "structural" not "behavioral" - exclude them.

**Detection heuristics**:
- Constructor: no `self` param, or name matches `new|default|from_.*|with_.*|into_.*`
- Accessor: reads exactly 1 field, writes 0, name matches `get_.*|is_.*|has_.*`
- Mutator: writes ≥1 field, name matches `set_.*|reset|clear`
- Worker: everything else

**Pros**:
- Directly addresses the false positive patterns
- Can provide richer feedback

**Cons**:
- Heuristic classification may be wrong
- More complex implementation
- Different structs may have different role distributions

---

## Comparison Matrix

| Option | Handles Constructors | Handles Builders | Catches Incohesive | Complexity | Interpretability |
|--------|---------------------|------------------|-------------------|------------|------------------|
| A: Field Coverage | Good | Good | Moderate | Low | High |
| B: Method-Field Density | Good | Moderate | Moderate | Low | Medium |
| C: LCOM4 | Good | Good | Good | Medium | Medium |
| D: Weighted Sharing | Good | Good | Good | Low | Medium |
| E: Pairwise Union | Moderate | Moderate | Good | Medium | Medium |
| F: Role-Aware | Excellent | Excellent | Good | High | High |

---

## Review Notes

### Available Data from Current Implementation

From `mdlr_core::Unit`:
- `reads: Vec<String>` - field names read via `self.field`
- `writes: Vec<String>` - field names written via `self.field = ...`
- `id: String` - fully qualified method name (e.g., `crate::module::Struct::method`)
- `parent: Option<String>` - struct ID this method belongs to
- `kind: UnitKind` - distinguishes `Method` vs `Function`

From `field_access.rs` extraction:
- Only tracks `self.field` patterns (not `self.inner.field` nested access)
- Method calls (`self.method()`) are correctly excluded from field access
- Chained access (`self.field.method()`) correctly captures `field` as a read
- Reads and writes are deduplicated per method

**Key limitation**: The extractor does not track field access in constructors that use struct literal syntax:
```rust
fn new() -> Self {
    Self { x: 0, y: 0 }  // x and y NOT tracked as writes
}
```
This is because there is no `self.x = ...` pattern. The `Self { ... }` struct literal is a different AST construct.

### Analysis by Option

#### Option A: Field Coverage Ratio

**Strengths:**
- Simple implementation: iterate fields, count how many methods access each
- Intuition is clear: "what fraction of fields are shared concerns"

**Issues:**
1. **Binary threshold problem**: A field accessed by 2 methods vs 10 methods both count as "shared". This loses granularity.
2. **Constructor masking**: With proper constructor tracking (if fixed), a single constructor writing all fields makes everything "shared", even if no two business methods share fields.
3. **Getter-only structs**: A struct with only getters (one per field) scores LCOM=1.0, which may be correct but could be surprising for data transfer objects.

**Edge case**: Struct with `new()` (writes all fields) + 5 getters (each reads 1 field). All fields are "shared" because `new` + one getter = 2 methods. LCOM = 0.0. But is this really cohesive? The getters have no relationship to each other.

#### Option B: Method-Field Density

**Strengths:**
- Captures "how much of the possible access matrix is filled"
- Easy to compute from existing data

**Issues:**
1. **Penalizes focused methods**: A well-designed method that only touches 1 of 10 fields contributes 0.1 to density. This punishes single-responsibility methods.
2. **Rewards god methods**: A method touching all fields maximizes density.
3. **Scale sensitivity**: A struct with 2 fields and 4 methods vs 10 fields and 4 methods have very different baselines.

**Example calculation issue**: The builder example shows LCOM=0.25, but the "incohesive" getter example shows LCOM=0.67. The gap may not be large enough to set meaningful thresholds.

#### Option C: LCOM4 (Connected Components)

**Strengths:**
- Well-established academic metric
- Transitive cohesion: if A shares with B, and B shares with C, they're all connected
- Constructor naturally connects everything (solves the false positive problem)
- Graph algorithms are well-understood; implementation is straightforward with BFS/DFS

**Issues:**
1. **Cliff effect**: A single shared field between two method clusters collapses multiple components into one. Going from LCOM4=2 to LCOM4=0 based on one field feels discontinuous.
2. **No gradient for "somewhat cohesive"**: Either methods are connected or not; no middle ground.
3. **Masking problem**: A utility method that touches all fields (logging, serialization) can connect disparate concerns.

**Implementation note**: The current codebase already groups methods by parent struct and iterates pairs. Converting to adjacency list + component counting adds ~20 lines of code using standard algorithms.

**Normalization question**: The document suggests `(components - 1) / (methods - 1)`. But consider a struct with 10 methods in 2 components: normalized = 1/9 = 0.11. A struct with 3 methods in 3 components: normalized = 2/2 = 1.0. The scale is hard to interpret. Perhaps `(components - 1) / max(1, methods - 1)` with special casing for 1-method structs.

#### Option D: Weighted Field Sharing

**Strengths:**
- Quadratic weighting rewards concentration: a field touched by 4 methods contributes 16, not 4
- Smooth gradient between cohesive and incohesive

**Issues:**
1. **max_possible formula seems wrong**: Document says `max_possible = methods² × fields`. Let's verify:
   - If every method touches every field: `access_count[f] = methods` for all f
   - `sharing_score = fields × methods²`
   - `cohesion = fields × methods² / (methods² × fields) = 1.0` ✓

   But wait, the formula counts `Σ (access_count[f])²`, not total accesses. If methods=4, fields=2, and each field is touched by all 4 methods: `sharing_score = 4² + 4² = 32`. `max_possible = 16 × 2 = 32`. This checks out.

2. **Sensitive to field count**: A struct with many fields that are individually accessed by few methods can score worse than a struct with few fields that are broadly accessed. Is that the desired behavior?

3. **Builder example gives LCOM=0.44**: This seems too high for a well-designed builder pattern. The quadratic penalty for fields touched by only 3/4 methods is harsh.

**Alternative consideration**: Use linear weighting instead of quadratic: `Σ access_count[f]` / `(methods × fields)`. This is essentially Option B (density) but field-centric.

#### Option E: Pairwise Field Union (Jaccard-like)

**Strengths:**
- Preserves the pairwise comparison structure of the current LCOM1 approach
- Accounts for method "scope": a narrow method paired with a broad method has a lower intersection/union ratio
- Bounded 0-1, same as current

**Issues:**
1. **Builder scores LCOM=0.5**: The document shows total_shared=6, total_union=12 for the builder example. This means even a well-structured builder is penalized significantly.
2. **Complexity**: O(m²) pairs × set operations. For large structs this may be slow, though likely acceptable.
3. **Disjoint setters problem**: Legitimate fluent builders or separate configuration methods (`set_timeout`, `set_retries`) that operate on disjoint fields will always contribute 0/union to the total, pulling LCOM up.

**Mathematical note**: This is like macro-averaged Jaccard similarity across all method pairs, inverted. It's more nuanced than binary "shares or not" but still penalizes focused methods.

#### Option F: Role-Aware LCOM

**Strengths:**
- Directly addresses the documented false positive patterns
- Can provide richer diagnostics ("LCOM 0.8 (excluding 3 constructors, 5 getters)")
- Most likely to match developer intuition

**Issues:**
1. **Heuristic fragility**: Name-based detection (`new`, `from_*`, `get_*`) is language/convention-specific and may false-positive/negative:
   - `new_connection()` is probably a constructor, but `renew()` is not
   - `get_or_insert()` is not a simple getter
   - `from_str()` trait impl may be a constructor or a parser

2. **Behavioral heuristics need more data**: The document suggests detecting constructors by "writes-only" but current extraction doesn't capture writes in struct literal syntax (`Self { ... }`). This makes the heuristic unreliable.

3. **What if all methods are "structural"?**: A pure data class with only constructors and getters would have 0 worker methods. LCOM is undefined or 0 by default?

4. **Implementation complexity**: Need to add classification logic, potentially expose it as a separate concern, handle edge cases for each category.

**Improvement to heuristics**:
- Constructor: no `self`/`&self`/`&mut self` parameter OR returns `Self`/`Self::*`
- Accessor: takes `&self`, returns something, body is essentially `self.field` or `self.field.clone()`
- Mutator: takes `&mut self`, has assignments to `self.field`
- The name-based fallback can supplement but shouldn't be primary

### Cross-Cutting Concerns

**1. Constructor field tracking gap**:
The biggest issue across all options is that constructors using `Self { field: value }` syntax don't register field writes. This affects:
- Option A: Fields may appear unshared when constructor should share them
- Option C: Constructor won't connect to other methods via shared fields
- Option D: Sharing scores will be lower than expected
- Option F: "Writes-only" heuristic won't work

**Fix required**: Extend `field_access.rs` to detect struct literal expressions and extract field names as writes when the struct name matches `Self` or the impl type.

**2. Associated functions vs methods**:
Currently, associated functions (no `self` parameter) like `fn new() -> Self` are still tracked as `UnitKind::Method` with a `parent`. They have empty `reads` and `writes` because there's no `self.field` to access. This is correct for the current extractor design, but it means:
- Constructors appear as "methods that touch no fields"
- They contribute to method count but not to field sharing
- In LCOM4, they become isolated nodes

**3. Trait implementations**:
Methods from trait impls (like `Display::fmt`) are grouped under the implementing struct. These may access fields differently than "native" methods:
- `fmt` typically reads all fields for display
- `Default::default` typically writes all fields
- `Clone::clone` reads all fields

Should trait impls be treated differently? Option F could classify them, but this adds more heuristic surface area.

**4. Generic and derived impls**:
Derived impls (`#[derive(Debug, Clone)]`) generate methods that touch all fields. These aren't extracted by tree-sitter (they're macro-expanded). If they were, they'd artificially inflate cohesion in Options A-E.

**5. Multiple impl blocks**:
A struct's methods can be spread across multiple `impl` blocks in different files. The current implementation correctly groups them by the resolved parent struct ID. All options should work correctly with this.

### Testing Considerations

To validate any chosen option, test against these patterns:

1. **Builder pattern**: Should score well (low LCOM)
2. **Service with constructor + focused methods**: Should score well
3. **Pure data struct (constructor + getters)**: Should score well or be exempt
4. **Facade grouping unrelated operations**: Should score poorly (high LCOM)
5. **State machine with disjoint state handlers**: May legitimately have high LCOM
6. **Generic container (`Vec<T>`-like)**: Methods operate on `self.inner`, should be cohesive

### Summary of Technical Gaps

| Issue | Affects Options | Severity |
|-------|-----------------|----------|
| Constructor struct literals not tracked | A, C, D, E | High |
| Associated functions have no field access | All | Medium |
| Name-based heuristics are fragile | F | Medium |
| Trait impl methods may skew results | All | Low |
| Derived impls not visible | All (positive bias) | Low |
