# mdlr - Code Modularity Analyzer

## Quick Start

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

## Workflow

1. Run `mdlr check` to identify modularity issues
2. Focus on high-value opportunities (top of each metric)
3. Drill down with `mdlr check <symbol>` to get metrics for a specific unit
4. Refactor to reduce complexity, coupling, and improve cohesion
5. Run `mdlr check --save` to cache results once satisfied

## Key Metrics

- **fan_out**: Dependencies a unit has. High = too many responsibilities
- **fan_in**: Units depending on this. Very high = potential bottleneck
- **function_size**: Lines of code in a function. High = hard to understand/test
- **file_loc**: Lines of code in a file. High = hard to navigate/maintain
- **cyclomatic**: Branch complexity. High = hard to test/maintain
- **lcom**: Lack of cohesion. High = struct should be split
- **methods_per_struct**: Methods in a struct. High = too many responsibilities
