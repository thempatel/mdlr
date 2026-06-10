use anyhow::{Result, bail};
use std::env;
use std::path::{Path, PathBuf};

use crate::cache::CacheStore;
use crate::cli::OutputFormat;
use crate::config;
use crate::display_scope::{self, DisplayScope};
use crate::extraction::{
    extract_go, extract_py, extract_rust, extract_ts, has_python_project,
    has_ts_files,
};
use crate::find_project_root;
use crate::progress::CheckProgress;
use crate::timing;
use mdlr_core::{Graph, Unit, UnitKind, build_with_progress as build_graph};
use mdlr_metrics::{
    ComplexityMetrics, CoverageMetrics, FileLocMetrics, LcovData,
    StructMetrics, StructuralMetrics,
    compute_with_hub_thresholds as compute_structural,
};

pub(crate) use crate::check_scope::{CheckFilter, ScopeInfo};
use crate::check_scope::{
    load_filtered_units, resolve_check_filter, resolve_filter_dir,
};

/// Bundle of all computed metrics for a graph
pub(crate) struct ComputedMetrics {
    pub(crate) graph: Graph,
    pub(crate) structural: StructuralMetrics,
    pub(crate) complexity: ComplexityMetrics,
    pub(crate) struct_metrics: StructMetrics,
    pub(crate) file_loc: FileLocMetrics,
    pub(crate) duplication: mdlr_cpd::DuplicationMetrics,
    pub(crate) coverage: Option<CoverageMetrics>,
}

/// Context for the check command, bundling common resources
struct CheckContext {
    cwd: std::path::PathBuf,
    store: CacheStore,
    config: config::Config,
    progress: CheckProgress,
    /// Generation ID (unix timestamp) shared across all extractors.
    /// Cache entries with `cached_at < generation_id` are stale.
    generation_id: u64,
}

impl CheckContext {
    fn new(explicit_root: Option<&Path>, quiet: bool) -> Result<Self> {
        let cwd = env::current_dir()?;
        let root = find_project_root(&cwd, explicit_root);
        let store = CacheStore::open(&root)?;
        let config = config::load_from_dir(store.root())?;
        let progress = CheckProgress::new(quiet);
        let generation_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Ok(CheckContext { cwd, store, config, progress, generation_id })
    }
}

/// Resolve the `--filter` folder and the run's `CheckFilter` from CLI args.
fn resolve_scope(
    args: &CheckArgs,
    ctx: &CheckContext,
) -> Result<(Option<PathBuf>, CheckFilter)> {
    let folder = resolve_filter_dir(args.filter.as_deref(), &ctx.cwd)?;
    let filter = resolve_check_filter(
        args.target.as_deref(),
        args.all,
        &ctx.cwd,
        ctx.store.root(),
    )?;
    Ok((folder, filter))
}

/// Emit hazard warnings about suspicious coverage results: no source files
/// matched, mostly-missing data, or no branch records.
fn warn_coverage_anomalies(progress: &CheckProgress, cov: &CoverageMetrics) {
    if cov.units_analyzed > 0
        && cov.lcov_files_total > 0
        && cov.lcov_files_matched == 0
    {
        progress.warn(&format!(
            "lcov references {} file(s) but none match any analyzed source — check that SF: paths point at source files mdlr sees (often a sourcemap issue: lcov references built .js while graph holds .ts, or paths are rooted differently than --root)",
            cov.lcov_files_total
        ));
    } else if cov.units_analyzed > 0
        && cov.units_without_data * 2 >= cov.units_analyzed
    {
        progress.warn(&format!(
            "{}/{} analyzed units had no coverage data — is the lcov file stale or incomplete?",
            cov.units_without_data, cov.units_analyzed
        ));
    }
    if !cov.has_branches {
        progress.warn(
            "lcov has no BRDA records — uncov_branches omitted (re-run coverage with branch instrumentation: c8 --all, coverage run --branch, llvm-cov --branch)",
        );
    }
}

/// Load `--cov` lcov files and compute coverage. Returns `None` when no
/// coverage files were passed or both coverage metrics are disabled.
fn compute_coverage(
    graph: &Graph,
    cov_files: &[PathBuf],
    repo_root: &Path,
    config: &config::Config,
    progress: &CheckProgress,
) -> Option<CoverageMetrics> {
    let coverage_disabled =
        config.is_disabled("line_cov") && config.is_disabled("uncov_branches");
    if cov_files.is_empty() || coverage_disabled {
        return None;
    }

    let spinner = progress.start_spinner("Loading coverage");
    let mut lcov = LcovData::new();
    let mut load_warnings: Vec<String> = Vec::new();
    for path in cov_files {
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            repo_root.join(path)
        };
        if let Err(e) = lcov.parse_and_merge(&resolved, repo_root) {
            load_warnings
                .push(format!("skipped --cov {}: {e}", resolved.display()));
        }
    }
    if load_warnings.is_empty() {
        spinner.finish();
    } else {
        spinner.finish_warn("partial");
    }
    for w in &load_warnings {
        progress.warn(w);
    }

    let cov = CoverageMetrics::compute(graph, &lcov, repo_root, None);
    warn_coverage_anomalies(progress, &cov);
    Some(cov)
}

#[tracing::instrument(name = "compute_metrics", skip_all)]
fn compute_all_metrics(
    units: Vec<Unit>,
    all_tokens: &[mdlr_cpd::FileTokens],
    config: &config::Config,
    progress: &CheckProgress,
    cov_files: &[PathBuf],
    repo_root: &Path,
) -> ComputedMetrics {
    let unit_count = units.len() as u64;
    let bar = progress.start_bar("Building graph", unit_count);
    let graph = tracing::info_span!("build_graph")
        .in_scope(|| build_graph(units, |i| bar.set_position(i as u64)));
    bar.finish();

    let total = graph.units.len() as u64;
    let bar = progress.start_bar("Computing metrics", total * 4);
    let structural = compute_structural(
        &graph,
        config.hub.min_fan_in,
        config.hub.min_fan_out,
        |i| bar.set_position(i as u64),
    );
    let complexity = ComplexityMetrics::compute_with_progress(&graph, |i| {
        bar.set_position(total + i as u64)
    });
    let struct_metrics = StructMetrics::compute_with_progress(&graph, |i| {
        bar.set_position(total * 2 + i as u64)
    });
    let file_loc = FileLocMetrics::compute_with_progress(&graph, |i| {
        bar.set_position(total * 3 + i as u64)
    });
    bar.finish();

    // CPD is the expensive pass; skip it entirely when duplication_pct is off.
    let duplication = if config.is_disabled("duplication_pct") {
        mdlr_cpd::DuplicationMetrics::default()
    } else {
        // Duplicated lines attribute to the innermost containing unit; whole-
        // file Module units are excluded so they don't swallow orphan lines
        // (duplicated imports/headers), which are dropped by design.
        let unit_spans: Vec<mdlr_cpd::UnitSpan> = graph
            .units
            .iter()
            .filter(|u| u.kind != UnitKind::Module)
            .map(|u| mdlr_cpd::UnitSpan {
                id: u.id.clone(),
                file: u.file.clone(),
                start_line: u.span.start_line as u32,
                end_line: u.span.end_line as u32,
            })
            .collect();
        let token_file_count = all_tokens.len() as u64;
        let bar = progress.start_bar("Detecting duplicates", token_file_count);
        let duplication = tracing::info_span!("cpd").in_scope(|| {
            let clones = mdlr_cpd::find_clones_with_progress(
                all_tokens,
                config.cpd.min_tokens,
                |i| bar.set_position(i as u64),
            );
            mdlr_cpd::compute_duplication(&clones, &unit_spans)
        });
        bar.finish();
        duplication
    };

    let coverage =
        compute_coverage(&graph, cov_files, repo_root, config, progress);

    ComputedMetrics {
        graph,
        structural,
        complexity,
        struct_metrics,
        file_loc,
        duplication,
        coverage,
    }
}

/// Set up timing instrumentation if requested, returns a printer to call after work is done.
fn setup_timing(enabled: bool) -> Option<timing::TimingPrinter> {
    if !enabled {
        return None;
    }
    let (layer, printer) = timing::TimingLayer::new();
    let subscriber = tracing_subscriber::registry::Registry::default();
    use tracing_subscriber::layer::SubscriberExt;
    let subscriber = subscriber.with(layer);
    tracing::subscriber::set_global_default(subscriber)
        .expect("failed to set tracing subscriber");
    Some(printer)
}

/// Load cache entries and collect units matching the filter.
/// Entries with `cached_at < generation_id` are stale and skipped.
/// Also loads all token caches for CPD (which needs project-wide data).
///
/// In diff mode every unit is loaded (metrics need the full graph) and the
/// returned [`DisplayScope`] holds the Changed Units — those whose span
/// overlaps a changed line — plus the touched files for `file_loc`.
fn run_extractor(
    name: &str,
    progress: &CheckProgress,
    f: impl FnOnce() -> Result<bool>,
) {
    let spinner = progress.start_spinner(name);
    match f() {
        Ok(true) => spinner.finish(),
        Ok(false) => spinner.finish_warn("partial"),
        Err(e) => {
            spinner.finish_warn("failed");
            eprintln!("Warning: {name} failed: {e:#}");
        }
    }
}

/// Run every language extractor whose project markers are present.
fn run_extractors(ctx: &CheckContext, progress: &CheckProgress) {
    let root = ctx.store.root();
    type ExtractFn = fn(&CacheStore, u64) -> Result<bool>;
    let extractors: [(&str, bool, ExtractFn); 4] = [
        ("Extracting Rust", root.join("Cargo.toml").exists(), extract_rust),
        ("Extracting TypeScript", has_ts_files(root), extract_ts),
        ("Extracting Go", root.join("go.mod").exists(), extract_go),
        ("Extracting Python", has_python_project(root), extract_py),
    ];
    for (name, detected, extract) in extractors {
        if detected {
            run_extractor(name, progress, || {
                extract(&ctx.store, ctx.generation_id)
            });
        }
    }
}

/// Extract, load, validate, and compute all metrics.
fn extract_and_analyze(
    ctx: &CheckContext,
    filter: &CheckFilter,
    folder: Option<&Path>,
    progress: &CheckProgress,
    cov_files: &[PathBuf],
) -> Result<(ComputedMetrics, usize, Option<DisplayScope>)> {
    run_extractors(ctx, progress);

    let spinner = progress.start_spinner("Loading cache");
    let (entries, units, all_tokens, scope) =
        load_filtered_units(&ctx.store, filter, folder, ctx.generation_id)?;
    spinner.finish();

    if let CheckFilter::Symbol(symbol_id) = filter {
        if !units.iter().any(|u| u.id == *symbol_id) {
            bail!(
                "Symbol '{}' not found. Run 'mdlr ls' to see available symbols.",
                symbol_id
            );
        }
    }

    let entry_count = entries.len();
    let mut computed = compute_all_metrics(
        units,
        &all_tokens,
        &ctx.config,
        progress,
        cov_files,
        ctx.store.root(),
    );
    if let Some(scope) = &scope {
        display_scope::apply(&mut computed, scope);
    }
    Ok((computed, entry_count, scope))
}

/// Inputs for [`handle_check`], mirroring the CLI `check` subcommand plus the
/// global `--root` flag.
pub struct CheckArgs {
    pub target: Option<String>,
    pub k: i32,
    pub pretty: bool,
    pub format: OutputFormat,
    pub timing: bool,
    pub all: bool,
    pub filter: Option<String>,
    pub quiet: bool,
    pub cov: Vec<PathBuf>,
    pub root: Option<PathBuf>,
}

pub fn handle_check(args: CheckArgs) -> Result<()> {
    let printer = setup_timing(args.timing);
    let ctx = CheckContext::new(args.root.as_deref(), args.quiet)?;
    let (folder, filter) = resolve_scope(&args, &ctx)?;

    let (computed, entry_count, scope) = extract_and_analyze(
        &ctx,
        &filter,
        folder.as_deref(),
        &ctx.progress,
        &args.cov,
    )?;

    let result = crate::check_output::render(
        &computed,
        &ctx.config,
        &crate::check_output::RenderArgs {
            format: args.format,
            k: args.k,
            pretty: args.pretty,
            entry_count,
            filter: &filter,
            scope: scope.as_ref(),
        },
        &ctx.store,
    );

    if let Some(printer) = printer {
        printer.print();
    }

    result
}
