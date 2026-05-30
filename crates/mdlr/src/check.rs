use anyhow::{Result, bail};
use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use crate::cache::CacheStore;
use crate::cli::OutputFormat;
use crate::config;
use crate::extraction::{
    extract_go, extract_py, extract_rust, extract_ts, has_python_project,
    has_ts_files, load_entries_from_dir, load_tokens_from_dir,
};
use crate::find_project_root;
use crate::progress::CheckProgress;
use crate::timing;
use mdlr_core::{Graph, Unit, build_with_progress as build_graph};
use mdlr_metrics::{
    ComplexityMetrics, CoverageMetrics, FileLocMetrics, LcovData,
    StructMetrics, StructuralMetrics,
    compute_with_hub_thresholds as compute_structural,
};

/// Represents what type of filter was specified
pub(crate) enum CheckFilter {
    /// No filter - analyze entire project
    None,
    /// Filter by file path
    File(PathBuf),
    /// Filter by directory path
    Directory(PathBuf),
    /// Filter by symbol ID
    Symbol(String),
    /// Filter by git diff — only files changed on the current branch
    Diff(HashSet<PathBuf>),
}

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
    /// Generation ID (unix timestamp) shared across all extractors.
    /// Cache entries with `cached_at < generation_id` are stale.
    generation_id: u64,
}

impl CheckContext {
    fn new(explicit_root: Option<&Path>) -> Result<Self> {
        let cwd = env::current_dir()?;
        let root = find_project_root(&cwd, explicit_root);
        let store = CacheStore::open(&root)?;
        let config = config::load_from_dir(store.root())?;
        let generation_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Ok(CheckContext { cwd, store, config, generation_id })
    }
}

/// Parse target string into a CheckFilter
fn parse_check_filter(target: Option<&str>, cwd: &Path) -> CheckFilter {
    if let Some(target_str) = target {
        let target_path = if Path::new(target_str).is_absolute() {
            Path::new(target_str).to_path_buf()
        } else {
            cwd.join(target_str)
        };

        if target_path.exists() {
            let canonical = target_path.canonicalize().unwrap_or(target_path);
            if canonical.is_file() {
                CheckFilter::File(canonical)
            } else {
                CheckFilter::Directory(canonical)
            }
        } else {
            CheckFilter::Symbol(target_str.to_string())
        }
    } else {
        CheckFilter::None
    }
}

/// Check if a file path passes the filter.
/// When `folder` is set, also requires the file to be inside that directory.
fn passes_path_filter(
    file_path: &Path,
    filter: &CheckFilter,
    folder: Option<&Path>,
) -> bool {
    let passes_mode = match filter {
        CheckFilter::File(filter_path) => file_path == *filter_path,
        CheckFilter::Directory(filter_path) => {
            file_path.starts_with(filter_path)
        }
        CheckFilter::Diff(changed) => {
            file_path.canonicalize().map_or(false, |p| changed.contains(&p))
        }
        CheckFilter::Symbol(_) | CheckFilter::None => true,
    };
    if !passes_mode {
        return false;
    }
    if let Some(folder) = folder {
        file_path.canonicalize().map_or(false, |p| p.starts_with(folder))
    } else {
        true
    }
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
    scope_files: Option<&HashSet<PathBuf>>,
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

    let cov = CoverageMetrics::compute(graph, &lcov, repo_root, scope_files);
    warn_coverage_anomalies(progress, &cov);
    Some(cov)
}

#[tracing::instrument(name = "compute_metrics", skip_all)]
fn compute_all_metrics(
    units: Vec<Unit>,
    all_tokens: &[mdlr_cpd::FileTokens],
    scope_files: Option<&HashSet<PathBuf>>,
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
        let token_file_count = all_tokens.len() as u64;
        let bar = progress.start_bar("Detecting duplicates", token_file_count);
        let duplication = tracing::info_span!("cpd").in_scope(|| {
            let clones = mdlr_cpd::find_clones_with_progress(
                all_tokens,
                config.cpd.min_tokens,
                |i| bar.set_position(i as u64),
            );
            mdlr_cpd::compute_duplication(&clones, all_tokens, scope_files)
        });
        bar.finish();
        duplication
    };

    let coverage = compute_coverage(
        &graph,
        cov_files,
        repo_root,
        scope_files,
        config,
        progress,
    );

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
fn load_filtered_units(
    store: &CacheStore,
    filter: &CheckFilter,
    folder: Option<&Path>,
    generation_id: u64,
) -> Result<(
    Vec<crate::cache::FileCacheEntry>,
    Vec<Unit>,
    Vec<mdlr_cpd::FileTokens>,
    Option<HashSet<PathBuf>>,
)> {
    let mut all_entries = Vec::new();
    load_entries_from_dir(&store.cache_dir(), &mut all_entries)?;

    let mut all_tokens = Vec::new();
    load_tokens_from_dir(&store.cache_dir(), &mut all_tokens)?;

    // Filter stale token caches
    all_tokens.retain(|t| t.cached_at >= generation_id);

    let mut entries = Vec::new();
    let mut units = Vec::new();
    let mut scope_files: Option<HashSet<PathBuf>> = match filter {
        CheckFilter::None => None,
        _ => Some(HashSet::new()),
    };

    for entry in all_entries {
        if entry.cached_at < generation_id {
            continue; // stale entry from a previous extraction
        }
        let file_path = store.root().join(&entry.source_path);
        if passes_path_filter(&file_path, filter, folder) {
            units.extend(entry.units.clone());
            if let Some(ref mut scope) = scope_files {
                scope.insert(entry.source_path.clone());
            }
        }
        entries.push(entry);
    }

    Ok((entries, units, all_tokens, scope_files))
}

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

/// Extract, load, validate, and compute all metrics.
fn extract_and_analyze(
    ctx: &CheckContext,
    filter: &CheckFilter,
    folder: Option<&Path>,
    progress: &CheckProgress,
    cov_files: &[PathBuf],
) -> Result<(ComputedMetrics, usize)> {
    let root = ctx.store.root();

    if root.join("Cargo.toml").exists() {
        run_extractor("Extracting Rust", progress, || {
            extract_rust(&ctx.store, ctx.generation_id)
        });
    }
    if has_ts_files(root) {
        run_extractor("Extracting TypeScript", progress, || {
            extract_ts(&ctx.store, ctx.generation_id)
        });
    }
    if root.join("go.mod").exists() {
        run_extractor("Extracting Go", progress, || {
            extract_go(&ctx.store, ctx.generation_id)
        });
    }
    if has_python_project(root) {
        run_extractor("Extracting Python", progress, || {
            extract_py(&ctx.store, ctx.generation_id)
        });
    }

    let spinner = progress.start_spinner("Loading cache");
    let (entries, units, all_tokens, scope_files) =
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
    let computed = compute_all_metrics(
        units,
        &all_tokens,
        scope_files.as_ref(),
        &ctx.config,
        progress,
        cov_files,
        ctx.store.root(),
    );
    Ok((computed, entry_count))
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
    let CheckArgs {
        target,
        k,
        pretty,
        format,
        timing,
        all,
        filter: filter_dir,
        quiet,
        cov,
        root,
    } = args;
    let target = target.as_deref();
    let filter_dir = filter_dir.as_deref();

    let printer = setup_timing(timing);
    let progress = CheckProgress::new(quiet);
    let ctx = CheckContext::new(root.as_deref())?;

    // Resolve --filter directory to a canonical path
    let folder = if let Some(dir) = filter_dir {
        let p = if Path::new(dir).is_absolute() {
            PathBuf::from(dir)
        } else {
            ctx.cwd.join(dir)
        };
        let canonical = p.canonicalize().map_err(|_| {
            anyhow::anyhow!("filter directory '{}' does not exist", dir)
        })?;
        if !canonical.is_dir() {
            bail!("filter path '{}' is not a directory", dir);
        }
        Some(canonical)
    } else {
        None
    };

    let filter = if target.is_some() || all {
        // Explicit target or --all flag: skip diff mode
        parse_check_filter(target, &ctx.cwd)
    } else if crate::git_diff::is_on_base_branch(ctx.store.root()) {
        // On main/master: only check staged + unstaged changes against HEAD
        let changed = crate::git_diff::diff_files_head(ctx.store.root())?;
        CheckFilter::Diff(changed)
    } else {
        // On a branch: diff mode by default
        let changed = crate::git_diff::diff_files(ctx.store.root())?;
        CheckFilter::Diff(changed)
    };

    let (computed, entry_count) = extract_and_analyze(
        &ctx,
        &filter,
        folder.as_deref(),
        &progress,
        &cov,
    )?;

    let result = match format {
        OutputFormat::Text => crate::check_output::format_text_output(
            &computed,
            &ctx.config,
            k,
            pretty,
            &filter,
            &ctx.store,
        ),
        OutputFormat::Json => crate::check_output::format_json_output(
            &computed,
            &ctx.config,
            entry_count,
            &filter,
        ),
    };

    if let Some(printer) = printer {
        printer.print();
    }

    result
}
