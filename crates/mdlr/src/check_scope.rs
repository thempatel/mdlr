//! Resolving what one `check` run reports on: the `CheckFilter` (explicit
//! target, `--filter` folder, or git-state-driven diff mode), the Changed
//! Units it selects, and the scope header describing it.

use anyhow::{Result, bail};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::cache::CacheStore;
use crate::display_scope::DisplayScope;
use crate::extraction::load_cache_dir;
use crate::git_diff::{ChangedFiles, WorkingState};
use crate::path_scope::PathScope;
use mdlr_core::Unit;

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

/// Resolve the `--filter` directory to a canonical path.
pub(crate) fn resolve_filter_dir(
    filter_dir: Option<&str>,
    cwd: &Path,
) -> Result<Option<PathBuf>> {
    let Some(dir) = filter_dir else { return Ok(None) };
    let p = if Path::new(dir).is_absolute() {
        PathBuf::from(dir)
    } else {
        cwd.join(dir)
    };
    let canonical = p.canonicalize().map_err(|_| {
        anyhow::anyhow!("filter directory '{}' does not exist", dir)
    })?;
    if !canonical.is_dir() {
        bail!("filter path '{}' is not a directory", dir);
    }
    Ok(Some(canonical))
}

/// Pick the run's filter. Diff-mode scope precedence: (1) a dirty working
/// tree (any source change vs HEAD — staged, unstaged, or untracked) scopes
/// to those edits' Changed Units; (2) a clean tree on a branch scopes to the
/// branch diff vs the merge-base; (3) a clean tree on main/master analyzes
/// the whole project.
pub(crate) fn resolve_check_filter(
    target: Option<&str>,
    all: bool,
    cwd: &Path,
    repo_root: &Path,
) -> Result<CheckFilter> {
    if target.is_some() || all {
        // Explicit target or --all flag: skip diff mode
        return Ok(parse_check_filter(target, cwd));
    }
    Ok(match crate::git_diff::classify_working_state(repo_root)? {
        WorkingState::Dirty(files) => {
            CheckFilter::Diff(DiffSpec { kind: DiffKind::Uncommitted, files })
        }
        WorkingState::OnBase => CheckFilter::None,
        WorkingState::Branch { base, files } => CheckFilter::Diff(DiffSpec {
            kind: DiffKind::Branch { base },
            files,
        }),
    })
}

/// Parse target string into a CheckFilter
pub(crate) fn parse_check_filter(
    target: Option<&str>,
    cwd: &Path,
) -> CheckFilter {
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
pub(crate) fn passes_path_filter(
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

pub(crate) fn load_filtered_units(
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
    let (all_entries, mut all_tokens) = load_cache_dir(&store.cache_dir())?;

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
        if let (CheckFilter::Diff(spec), Some(scope)) = (filter, &mut scope)
            && let Some(canonical) = canonical_in_folder(&file_path, folder)
        {
            collect_changed_units(spec, &entry, &canonical, scope);
        }
        entries.push(entry);
    }

    Ok((entries, units, all_tokens, scope))
}

/// The canonical form of `path`, if it passes the optional folder
/// restriction.
fn canonical_in_folder(path: &Path, folder: Option<&Path>) -> Option<PathBuf> {
    let canonical = path.canonicalize().ok()?;
    folder.is_none_or(|f| canonical.starts_with(f)).then_some(canonical)
}

/// Add `entry`'s Changed Units (span overlapping a changed line) and touched
/// file to the display scope. A unit is in scope if *any* changed line falls
/// in its span — all overlapping units count, parents included.
pub(crate) fn collect_changed_units(
    spec: &DiffSpec,
    entry: &crate::cache::FileCacheEntry,
    canonical_path: &Path,
    scope: &mut DisplayScope,
) {
    let Some(span) = spec.files.get(canonical_path) else { return };

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
    add_parent_closure(scope, &entry.units);
}

/// Close over parent pointers: a changed method puts its struct in scope
/// (its lcom/methods_per_struct genuinely changed) even though the struct's
/// span — just the field block in Rust — doesn't contain the changed lines.
fn add_parent_closure(scope: &mut DisplayScope, units: &[Unit]) {
    loop {
        let mut added = false;
        for unit in units {
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

/// Build the scope header line announcing what this run reports on.
pub(crate) fn describe_scope(
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
