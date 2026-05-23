# Claude Code Instructions

## Documentation

Documentation lives in `docs/` with the following structure:

```
docs/
├── README.md                 # Index linking to all docs
├── getting-started/          # Installation and quick start
├── metrics/                  # Metric explanations and interpretation
└── reference/                # CLI, graph structure, languages, sessions
```

### When to Update Docs

Update documentation when:

- **Adding a new metric**: Create `docs/metrics/<metric-name>.md` and add to `docs/metrics/overview.md` and `docs/README.md`
- **Adding a CLI command**: Update `docs/reference/cli.md`
- **Adding language support**: Update `docs/reference/languages.md`
- **Changing graph structure**: Update `docs/reference/graph.md`
- **Changing session storage**: Update `docs/reference/sessions.md`
- **Changing installation steps**: Update `docs/README.md`

### Documentation Style

- Use tables for structured information
- Include code examples with realistic output
- Keep explanations concise
- Link between related docs using relative paths
- Update `docs/README.md` index when adding new files

## Adding Features

### New Metric

1. Add computation to `src/metrics/structural.rs` or create new file
2. Export from `src/metrics/mod.rs`
3. Wire into `handle_analyze` in `src/main.rs`
4. **Add to the metrics list in `handle_metrics()` in `src/main.rs`** with a description explaining what high/low values indicate
5. Create `docs/metrics/<metric-name>.md`
6. Update `docs/metrics/overview.md` and `docs/README.md`

**Important**: If a metric's meaning or interpretation changes, update its description in `handle_metrics()` accordingly.

### New Language

1. Create a new extractor crate under `crates/` that outputs `FileCacheEntry`-compatible JSON
2. Wire orchestration into `crates/mdlr/src/main.rs`
3. Update `docs/reference/languages.md`

### New CLI Command

1. Add to `src/cli.rs` enums
2. Add handler in `src/main.rs`
3. Update `docs/reference/cli.md`
4. Update `src/prompt.md` if the command affects the typical workflow

### Updating Functionality

When changing CLI behavior, flags, or workflow:

- Update `src/prompt.md` to reflect the changes (this is the agent-facing prompt output by `mdlr prompt`)

## Running mdlr

* **IMPORTANT**: Prefer to run `mdlr` as a direct binary, not via `cargo run`.
* **IMPORTANT**: Use `task link` to build `mdlr` and the related tools.
