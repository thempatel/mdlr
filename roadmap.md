# mdlr Roadmap

## Phase 1: Barebones Foundation ✓

Core infrastructure for modularity analysis.

- [x] Project structure (cli, graph, session, extract, metrics modules)
- [x] Core data types (Unit, Edge, Graph, Span)
- [x] Session system with file-based persistence
- [x] CLI with subcommands (session, target, analyze, export)
- [x] Extractor trait for language-agnostic parsing
- [x] Rust extractor (functions, structs, traits, impl blocks, calls)
- [x] Structural metrics (dag_density, fan_in, fan_out)
- [x] Documentation

## Phase 2: Language Support

Add extractors for additional languages.

- [ ] TypeScript extractor (`.ts`, `.tsx`)
- [ ] Go extractor (`.go`)
- [ ] Python extractor (`.py`)

## Phase 3: Semantic Metrics

Metrics that capture higher-level modularity concerns.

- [ ] `concept_scatter` - How spread out is a concept across the codebase?
- [ ] `closure` - What's the transitive closure of dependencies?
- [ ] `edge_cut_ratio` - How cleanly can the graph be partitioned?

## Phase 4: Workflow Features

Features for integrating into development workflows.

- [ ] Diff mode - Compare metrics between commits/branches
- [ ] Watch mode - Continuous analysis during development
- [ ] Visualization export - Generate graphs for external tools (DOT, Mermaid)

## Phase 5: Entity Inference

Better detection of data flow relationships.

- [ ] Reads analysis - Track which entities a unit consumes
- [ ] Writes analysis - Track which entities a unit produces
- [ ] Data flow edges - Connect producers to consumers
