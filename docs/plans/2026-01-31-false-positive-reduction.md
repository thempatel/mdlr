---
title: False Positive Reduction Plan
date: 2026-01-31
status: draft
---

# False Positive Reduction Plan

This document outlines options for making mdlr metrics more robust against false positives.

## Background

The current metrics system produces false positives in several categories:

1. **LCOM (Lack of Cohesion)**: Data structs with constructors, builders, and service patterns are flagged as incohesive when the design is intentional
2. **Fan-In**: Test utilities inflate metrics
3. **Call Resolution**: Ambiguous name resolution, macro invocations
4. **Cross-Crate**: Unresolved external dependencies

Current mitigation: Manual per-symbol ignores via `mdlr ignore <metric> "<symbol>"` stored in `.mdlr/ignores.json`.

## Proposed Options

### Option 1: Constructor Detection for LCOM

**Problem**: LCOM penalizes constructors that write fields but don't read them.

**Solution**: Detect and exclude constructor-like methods from LCOM calculation.

Detection heuristics:
- Method name: `new`, `default`, `build`, `from_*`, `with_*`, `into_*`
- No `self` parameter (static method)
- Returns `Self` or the struct type

**Implementation**:
- Modify `compute_struct_lcom` in `crates/mdlr-metrics/src/struct_metrics.rs`
- Add `is_constructor_like(method: &Unit) -> bool` helper
- Filter constructor methods before pair comparison

**Trade-offs**:
- (+) Simple, addresses most common LCOM false positive
- (+) No configuration required
- (-) May miss custom constructor patterns
- (-) Heuristic-based

**Complexity**: Low

---

### Option 2: Path-Based Filtering

**Problem**: Test code and generated code inflate metrics.

**Solution**: Add configurable path patterns to exclude from analysis.

```toml
# .mdlr.toml
[filters]
exclude_paths = [
    "**/tests/**",
    "**/test_*.rs",
    "**/*_test.rs",
    "**/generated/**",
]
```

**Implementation**:
- Add `filters` section to `Config` in `crates/mdlr/src/config/types.rs`
- Apply glob matching in `SourceWalker::walk()` in `crates/mdlr/src/walk.rs`

**Trade-offs**:
- (+) User controls what's excluded
- (+) Works for any file category
- (-) Requires per-project configuration

**Complexity**: Low

---

### Option 3: Minimum Method Threshold for LCOM

**Problem**: LCOM is noisy for structs with few methods.

**Solution**: Only compute/report LCOM for structs with >= N methods.

```toml
[thresholds]
lcom_min_methods = 4
```

**Implementation**:
- Add `lcom_min_methods` to `ThresholdsConfig`
- Check in `compute_struct_lcom` before processing

**Trade-offs**:
- (+) Simple to implement
- (+) Reduces noise from trivial structs
- (-) Arbitrary cutoff

**Complexity**: Low

---

### Option 4: Wildcard Ignore Patterns

**Problem**: Current ignores are per-symbol, tedious for patterns.

**Solution**: Support glob patterns in ignores.

```bash
mdlr ignore lcom "**::new"           # All constructors
mdlr ignore lcom "*::Builder"        # All builders
mdlr ignore fan_in "**::test_*"      # Test functions
```

**Implementation**:
- Extend `Ignores` struct in `crates/mdlr/src/cache/ignores_store.rs`
- Add `IgnoreEntry` enum with `Exact(String)` and `Pattern(String)` variants
- Use `glob` crate for pattern matching in `is_ignored`

**Trade-offs**:
- (+) Dramatically reduces manual effort
- (+) Familiar syntax
- (-) Patterns may match unintended symbols

**Complexity**: Medium

---

### Option 5: Macro Filtering in Call Extraction

**Problem**: Macro invocations may appear in call lists.

**Solution**: Detect and exclude macro calls during extraction.

Detection: Tree-sitter `macro_invocation` nodes (or calls ending in `!`)

**Implementation**:
- Modify `extract_calls` in `crates/mdlr-extract-rust/src/extractor.rs`
- Filter out `macro_invocation` nodes

**Trade-offs**:
- (+) Removes common noise source
- (+) Tree-sitter already identifies macros
- (-) Some macros expand to real function calls

**Complexity**: Low

---

### Option 6: Explain Mode

**Problem**: Users need to understand why a metric is flagged.

**Solution**: Add `--explain` flag showing metric breakdown.

```bash
$ mdlr check my_crate::Builder --explain

my_crate::Builder LCOM = 0.85 (poor)
  Methods: new, with_name, with_value, build
  Field access:
    - new: writes [name, value]
    - with_name: writes [name]
    - with_value: writes [value]
    - build: reads [name, value]
  Pattern: Builder (methods chain writes, final method reads)
  Suggestion: Consider ignoring LCOM for builder types
```

**Implementation**:
- Add `--explain` flag to CLI in `crates/mdlr/src/cli.rs`
- Add detailed breakdown output in check handler
- Detect common patterns (builder, data struct, factory)

**Trade-offs**:
- (+) Educational, aids debugging
- (+) Helps users make informed decisions
- (-) More complex output logic

**Complexity**: Medium

---

### Option 7: Auto-Detect Patterns and Suggest Ignores

**Problem**: Users don't know when a flag is a false positive.

**Solution**: When a violation matches a known pattern, suggest ignoring.

```
lcom  my_crate::Builder  0.85  poor
      ^ Builder pattern detected. Suppress: mdlr ignore lcom "my_crate::Builder"
```

Patterns to detect:
- **Builder**: Methods named `with_*`, `set_*`, `build`
- **Data struct**: Public fields, few methods, constructor only
- **Factory**: Multiple methods returning `Self`
- **Service**: Different methods access different subsets of fields

**Implementation**:
- Add pattern matchers in metrics output
- Emit suggestions alongside violations

**Trade-offs**:
- (+) Guides users to correct action
- (+) Encodes best practices
- (-) Pattern detection is heuristic

**Complexity**: Medium

---

### Option 8: Resolution Confidence Tracking

**Problem**: Some edges are guesses when resolution is ambiguous.

**Solution**: Track confidence level of each resolved call.

Levels:
- **Resolved**: Fully qualified path determined
- **Inferred**: Likely match based on context
- **Ambiguous**: Multiple candidates, picked first match

**Implementation**:
- Add `ResolutionConfidence` enum
- Return confidence from resolution functions
- Optionally filter metrics by confidence

**Trade-offs**:
- (+) Transparency about edge quality
- (+) Can filter low-confidence edges
- (-) Adds complexity to graph model

**Complexity**: Medium

---

### Option 9: LCOM4 Algorithm

**Problem**: Current LCOM doesn't handle transitive cohesion.

**Solution**: Use LCOM4 which builds a graph of method relationships.

Algorithm:
1. Create node per method
2. Connect methods that share field access
3. LCOM4 = number of connected components - 1

**Trade-offs**:
- (+) Standard metric with known behavior
- (+) Naturally handles some edge cases better
- (-) Different interpretation than current
- (-) Requires migration documentation

**Complexity**: Medium

---

## Recommended Phases

### Phase 1: Quick Wins (Low effort, High impact)
1. **Constructor detection** (Option 1) - Addresses most common LCOM false positive
2. **Path-based filtering** (Option 2) - Removes test code from analysis
3. **Minimum method threshold** (Option 3) - Reduces noise from trivial structs
4. **Macro filtering** (Option 5) - Cleaner call extraction

### Phase 2: Enhanced Ignores
5. **Wildcard patterns** (Option 4) - Scalable suppression
6. **Explain mode** (Option 6) - Better debugging

### Phase 3: Smarter Detection
7. **Auto-suggest ignores** (Option 7) - Guided user experience
8. **Resolution confidence** (Option 8) - Edge quality tracking

### Phase 4: Algorithm Improvements
9. **LCOM4** (Option 9) - Better cohesion measurement

## Critical Files

| File | Changes |
|------|---------|
| `crates/mdlr-metrics/src/struct_metrics.rs` | Options 1, 3, 9 |
| `crates/mdlr/src/config/types.rs` | Options 2, 3 |
| `crates/mdlr/src/walk.rs` | Option 2 |
| `crates/mdlr/src/cache/ignores_store.rs` | Option 4 |
| `crates/mdlr-extract-rust/src/extractor.rs` | Option 5 |
| `crates/mdlr/src/cli.rs` | Option 6 |
| `crates/mdlr/src/main.rs` | Options 6, 7 |

## Verification

After implementation:
1. Run `mdlr analyze` on mdlr itself - known false positives in `docs/roadmap/ignored-metrics.md` should not appear
2. Run `mdlr check` - no regressions in detection of real issues
3. Test path filtering with `--exclude` on test files
4. Verify wildcard ignores match expected symbols
