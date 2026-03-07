# Auto-Improve

Use mdlr to identify and improve modularity issues in the codebase.

**IMPORTANT**: Never use cargo run. Always use `task link` to build `mdlr` and then use `mdlr` directly as a binary.

## mdlr Reference

### Quick Start

```bash
# Analyze codebase and show top opportunities per metric
mdlr check

# Analyze specific directory or file
mdlr check src/metrics
mdlr check src/main.rs

# Analyze a specific symbol by fully qualified crate name
mdlr check "mdlr::handle_check"
mdlr check "mdlr::cache::store::CacheStore"

# Show more results per metric
mdlr check -k 10

# Pretty print as aligned table
mdlr check --pretty

# List available metrics and their meanings
mdlr metrics ls

# Get details about a specific metric including thresholds
mdlr metrics get cyclomatic
```

### Key Metrics

- **fan_out**: Dependencies a unit has. High = too many responsibilities
- **fan_in**: Units depending on this. Very high = potential bottleneck
- **function_size**: Lines of code in a function. High = hard to understand/test
- **file_loc**: Lines of code in a file. High = hard to navigate/maintain
- **cyclomatic**: Branch complexity. High = hard to test/maintain
- **lcom**: Lack of cohesion. High = struct should be split
- **methods_per_struct**: Methods in a struct. High = too many responsibilities

## Steps

1. Run all unit tests.
2. Run `mdlr check` to identify modularity issues
3. Focus on high-value opportunities (top of each metric)
4. Drill down with `mdlr check <symbol>` to get metrics for a specific unit
5. Create a plan and consider alternatives before making changes
6. Follow the plan to make the suggested improvements to the codebase
7. Ensure all existing tests continue to pass by running `cargo test`
8. Update or add tests as needed to cover your changes
9. If you add a new metric, CLI command, or language support, update the relevant documentation as specified in CLAUDE.md

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
