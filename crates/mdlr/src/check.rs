use anyhow::{Result, bail};
use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use crate::cache::CacheStore;
use crate::cli::OutputFormat;
use crate::config;
use crate::display_scope::{self, DisplayScope};
use crate::extraction::{
    extract_go, extract_py, extract_rust, extract_ts, has_python_project,
    has_ts_files, load_entries_from_dir, load_tokens_from_dir,
};
use crate::find_project_root;
use crate::git_diff::ChangedFiles;
use crate::path_scope::PathScope;
use crate::progress::CheckProgress;
use crate::timing;
use mdlr_core::{Graph, Unit, UnitKind, build_with_progress as build_graph};
use mdlr_metrics::{
    ComplexityMetrics, CoverageMetrics, FileLocMetrics, LcovData,
    StructMetrics, StructuralMetrics,
    compute_with_hub_thresholds as compute_structural,
};

/// Represents what type of filter was specified
pub(crate) enum CheckFilter {
    /// No filter - analyze entire project
    None,
    /// Filter by a file or directory path
    Path(PathScope),
    /// Filter by symbol ID
    Symbol(String),
    /// Diff mode — display only Units whose span overlaps a changed line
    Diff(DiffSpec),
}

/// The active diff for diff mode: which lines changed, and relative to what.
pub(crate) struct DiffSpec {
    pub kind: DiffKind,
    pub files: ChangedFiles,
}

pub(crate) enum DiffKind {
    /// Working tree vs HEAD (staged + unstaged + untracked).
    Uncommitted,
    /// Branch vs its merge-base with the base branch.
    Branch { base: String },
}

/// Scope description for the output header, since diff mode switches scopes
/// silently on git state.
pub(crate) struct ScopeInfo {
    pub mode: &'static str,
    pub description: String,
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
    match target {
        Some(target_str) => {
            match PathScope::classify(Path::new(target_str), cwd) {
                Some(scope) => CheckFilter::Path(scope),
                None => CheckFilter::Symbol(target_str.to_string()),
            }
        }
        None => CheckFilter::None,
    }
}

/// Check if a file path passes the filter.
/// When `folder` is set, also requires the file to be inside that directory.
/// Diff mode never load-filters: all units stay in the graph so metric values
/// (fan_in in particular) are accurate, and scoping happens at display time.
fn passes_path_filter(
    file_path: &Path,
    filter: &CheckFilter,
    folder: Option<&Path>,
) -> bool {
    let passes_mode = match filter {
        CheckFilter::Path(scope) => scope.matches(file_path),
        CheckFilter::Symbol(_) | CheckFilter::None | CheckFilter::Diff(_) => {
            true
        }
    };
    if !passes_mode {
        return false;
    }
    match filter {
        // Diff mode: the folder restricts the display scope, not the graph.
        CheckFilter::Diff(_) => true,
        _ => match folder {
            Some(folder) => file_path
                .canonicalize()
                .map_or(false, |p| p.starts_with(folder)),
            None => true,
        },
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
fn load_filtered_units(
    store: &CacheStore,
    filter: &CheckFilter,
    folder: Option<&Path>,
    generation_id: u64,
) -> Result<(
    Vec<crate::cache::FileCacheEntry>,
    Vec<Unit>,
    Vec<mdlr_cpd::FileTokens>,
    Option<DisplayScope>,
)> {
    let mut all_entries = Vec::new();
    load_entries_from_dir(&store.cache_dir(), &mut all_entries)?;

    let mut all_tokens = Vec::new();
    load_tokens_from_dir(&store.cache_dir(), &mut all_tokens)?;

    // Filter stale token caches
    all_tokens.retain(|t| t.cached_at >= generation_id);

    let mut entries = Vec::new();
    let mut units = Vec::new();
    let mut scope: Option<DisplayScope> = match filter {
        CheckFilter::Diff(_) => Some(DisplayScope {
            unit_ids: HashSet::new(),
            files: HashSet::new(),
            touched_files: 0,
        }),
        _ => None,
    };

    for entry in all_entries {
        if entry.cached_at < generation_id {
            continue; // stale entry from a previous extraction
        }
        let file_path = store.root().join(&entry.source_path);
        if passes_path_filter(&file_path, filter, folder) {
            units.extend(entry.units.clone());
        }
        if let (CheckFilter::Diff(spec), Some(scope)) = (filter, &mut scope) {
            collect_changed_units(spec, &entry, &file_path, folder, scope);
        }
        entries.push(entry);
    }

    Ok((entries, units, all_tokens, scope))
}

/// Add `entry`'s Changed Units (span overlapping a changed line) and touched
/// file to the display scope. A unit is in scope if *any* changed line falls
/// in its span — all overlapping units count, parents included.
fn collect_changed_units(
    spec: &DiffSpec,
    entry: &crate::cache::FileCacheEntry,
    file_path: &Path,
    folder: Option<&Path>,
    scope: &mut DisplayScope,
) {
    let Ok(canonical) = file_path.canonicalize() else { return };
    if let Some(folder) = folder
        && !canonical.starts_with(folder)
    {
        return;
    }
    let Some(span) = spec.files.get(&canonical) else { return };

    scope.touched_files += 1;
    // `file_loc` keys rows by the unit's `file` string; record the entry's
    // source path too in case the entry has no units.
    scope.files.insert(entry.source_path.to_string_lossy().to_string());
    for unit in &entry.units {
        scope.files.insert(unit.file.to_string_lossy().to_string());
        if span.overlaps(unit.span.start_line, unit.span.end_line) {
            scope.unit_ids.insert(unit.id.clone());
        }
    }
    // Close over parent pointers: a changed method puts its struct in scope
    // (its lcom/methods_per_struct genuinely changed) even though the struct's
    // span — just the field block in Rust — doesn't contain the changed lines.
    loop {
        let mut added = false;
        for unit in &entry.units {
            if scope.unit_ids.contains(&unit.id)
                && let Some(parent) = &unit.parent
            {
                added |= scope.unit_ids.insert(parent.clone());
            }
        }
        if !added {
            break;
        }
    }
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

/// Build the scope header line announcing what this run reports on.
fn describe_scope(
    filter: &CheckFilter,
    scope: Option<&DisplayScope>,
) -> ScopeInfo {
    match filter {
        CheckFilter::None => ScopeInfo {
            mode: "whole-project",
            description: "whole project".to_string(),
        },
        CheckFilter::Path(p) => {
            let path = match p {
                PathScope::File(p) | PathScope::Directory(p) => p.display(),
            };
            ScopeInfo { mode: "path", description: format!("path {path}") }
        }
        CheckFilter::Symbol(s) => {
            ScopeInfo { mode: "symbol", description: format!("symbol {s}") }
        }
        CheckFilter::Diff(spec) => {
            let (mode, what) = match &spec.kind {
                DiffKind::Uncommitted => {
                    ("uncommitted", "uncommitted changes".to_string())
                }
                DiffKind::Branch { base } => {
                    ("branch-diff", format!("branch diff vs {base}"))
                }
            };
            let (units, files) = scope
                .map(|s| (s.unit_ids.len(), s.touched_files))
                .unwrap_or((0, 0));
            ScopeInfo {
                mode,
                description: format!(
                    "{what} ({units} unit{} in {files} file{})",
                    if units == 1 { "" } else { "s" },
                    if files == 1 { "" } else { "s" },
                ),
            }
        }
    }
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

    // Diff-mode scope precedence: (1) a dirty working tree (any source change
    // vs HEAD — staged, unstaged, or untracked) scopes to those edits' Changed
    // Units; (2) a clean tree on a branch scopes to the branch diff vs the
    // merge-base; (3) a clean tree on main/master analyzes the whole project.
    let filter = if target.is_some() || all {
        // Explicit target or --all flag: skip diff mode
        parse_check_filter(target, &ctx.cwd)
    } else {
        let dirty = crate::git_diff::working_tree_changes(ctx.store.root())?;
        if dirty.keys().any(|p| crate::extraction::is_source_path(p)) {
            CheckFilter::Diff(DiffSpec {
                kind: DiffKind::Uncommitted,
                files: dirty,
            })
        } else if crate::git_diff::is_on_base_branch(ctx.store.root()) {
            CheckFilter::None
        } else {
            let (base, files) =
                crate::git_diff::branch_changes(ctx.store.root())?;
            CheckFilter::Diff(DiffSpec {
                kind: DiffKind::Branch { base },
                files,
            })
        }
    };

    let (computed, entry_count, scope) = extract_and_analyze(
        &ctx,
        &filter,
        folder.as_deref(),
        &progress,
        &cov,
    )?;

    let scope_info = describe_scope(&filter, scope.as_ref());

    let result = match format {
        OutputFormat::Text => crate::check_output::format_text_output(
            &computed,
            &ctx.config,
            k,
            pretty,
            &filter,
            &ctx.store,
            &scope_info,
        ),
        OutputFormat::Json => crate::check_output::format_json_output(
            &computed,
            &ctx.config,
            entry_count,
            &filter,
            &scope_info,
        ),
    };

    if let Some(printer) = printer {
        printer.print();
    }

    result
}
