# Auto-Improve

Use mdlr to identify and improve modularity issues in the codebase.

## mdlr Reference

### Quick Start

```bash
# Analyze codebase. Scope picks itself from git state and is announced by a
# "scope:" header line: units touched by uncommitted changes if the working
# tree is dirty; units changed vs main/master if on a clean branch; the whole
# project on clean main/master. Typical loop: edit -> mdlr check -> commit.
mdlr check

# Force all files even when on a branch
mdlr check -A

# Analyze specific directory or file
mdlr check src/metrics
mdlr check src/main.rs

# Scope to a folder (combines with diff/all mode)
mdlr check -f src/metrics
mdlr check -A -f src/metrics

# Analyze a specific symbol by fully qualified crate name
mdlr check "mdlr::handle_check"
mdlr check "mdlr::cache::store::CacheStore"

# Show more results per metric
mdlr check -k 10

# Pretty print as aligned table
mdlr check --pretty

# Overlay test coverage from an LCOV file (adds line_cov + uncov_branches metrics)
mdlr check --cov target/llvm-cov/lcov.info

# Merge multiple LCOV files (e.g. frontend + backend monorepo)
mdlr check --cov frontend/lcov.info --cov backend/lcov.info

# List available metrics and their meanings
mdlr metrics ls

# Get details about a specific metric including thresholds
mdlr metrics get cyclomatic
```

### Key Metrics

- **fan_out**: Dependencies a unit has. High = too many responsibilities
- **fan_in**: Units depending on this. Very high = potential bottleneck
- **function_size**: Lines of code in a function. Two-sided: high = hard to understand/test (split it); low (1-2 lines, flagged only when the function has exactly one caller) = a pass-through adding indirection without abstraction (inline it into its caller). Tiny functions with zero or multiple callers are never flagged — do not inline trait-required methods, public API accessors, or shared helpers
- **file_loc**: Lines of code in a file. High = hard to navigate/maintain
- **cyclomatic**: Branch complexity. High = hard to test/maintain
- **cognitive**: Nesting-aware complexity. High = hard to understand (penalizes deep nesting)
- **lcom**: Lack of cohesion. High = struct should be split
- **methods_per_struct**: Methods in a struct. High = too many responsibilities
- **duplication_pct**: % of a unit's lines that are copy-pasted (attributed to the innermost containing unit). High = extract a shared abstraction
- **line_cov** (only with `--cov`): Per-function test coverage %. LOW = untested (lower is worse)
- **uncov_branches** (only with `--cov` + BRDA): Per-function untaken branches. High = unexercised code paths

## Steps

1. Run all unit tests.
   - **Optional**: run the test suite in coverage mode (e.g. `cargo llvm-cov --lcov --output-path lcov.info`, `coverage run --branch` + `coverage lcov`, `c8 --reporter=lcov`) and pass the resulting LCOV file to `mdlr check --cov <path>` in the next step. This adds `line_cov` and `uncov_branches` to the metrics output so you can prioritize under-tested units alongside complexity hot-spots.
2. Run `mdlr check` to identify modularity issues (add `--cov lcov.info` if you generated coverage in step 1)
3. Focus on high-value opportunities (top of each metric)
4. Drill down with `mdlr check <symbol>` to get metrics for a specific unit
5. Create a plan and consider alternatives before making changes
6. Follow the plan to make the suggested improvements to the codebase
7. Ensure all existing tests continue to pass by running `cargo test`
8. Update or add tests as needed to cover your changes
9. If you add a new metric, CLI command, or language support, update the relevant documentation as specified in CLAUDE.md
10. **Commit your changes** (see Final Step below)

## Important: Choose the Best Fix

When fixing a modularity issue, there are often multiple valid approaches. Think critically about which solution produces the cleanest result:

- **Splitting**: Extract part of a function/struct into a helper. Good when there's a clear sub-responsibility.
- **Restructuring**: Redesign the approach so the complexity isn't needed. Often the best solution.
- **Consolidating**: Sometimes code is scattered and should be unified before being split differently.

For example, a large function might be fixed by:
1. Extracting helpers (reduces size but adds indirection)
2. Using a different algorithm that's inherently simpler
3. Moving some logic to callers where it belongs
4. Introducing a data structure that eliminates branching

Pick the approach that results in the cleanest, most maintainable code—not just the one that lowers the metric fastest.
