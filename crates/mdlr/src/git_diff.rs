//! Git interaction for `mdlr check`'s diff mode: detecting the base branch and
//! collecting the changed line ranges of the working tree / current branch.

use anyhow::{Result, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process;

/// The changed portion of one file, on the new side of the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChangedSpan {
    /// The entire file is new (untracked or added).
    WholeFile,
    /// Changed line ranges, inclusive (start, end), 1-based new-side lines.
    Lines(Vec<(usize, usize)>),
}

impl ChangedSpan {
    /// Whether a unit spanning `start..=end` overlaps this change.
    pub(crate) fn overlaps(&self, start: usize, end: usize) -> bool {
        match self {
            ChangedSpan::WholeFile => true,
            ChangedSpan::Lines(ranges) => {
                ranges.iter().any(|&(s, e)| s <= end && start <= e)
            }
        }
    }
}

/// Changed files keyed by canonical absolute path.
pub(crate) type ChangedFiles = HashMap<PathBuf, ChangedSpan>;

/// Check if the current HEAD is on the base branch (main or master).
/// The repo state that drives diff-mode scope precedence.
pub(crate) enum WorkingState {
    /// Source changes vs HEAD (staged, unstaged, or untracked).
    Dirty(ChangedFiles),
    /// Clean tree, sitting on main/master.
    OnBase,
    /// Clean tree on a branch: changes vs the merge-base with `base`.
    Branch { base: String, files: ChangedFiles },
}

/// Classify the working tree for scope selection: dirty source edits win,
/// then a clean tree is either on the base branch or carries a branch diff.
pub(crate) fn classify_working_state(root: &Path) -> Result<WorkingState> {
    let dirty = working_tree_changes(root)?;
    if dirty.keys().any(|p| crate::extraction::is_source_path(p)) {
        return Ok(WorkingState::Dirty(dirty));
    }
    if is_on_base_branch(root) {
        return Ok(WorkingState::OnBase);
    }
    let (base, files) = branch_changes(root)?;
    Ok(WorkingState::Branch { base, files })
}

pub(crate) fn is_on_base_branch(root: &Path) -> bool {
    let output = process::Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(root)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let branch = String::from_utf8_lossy(&o.stdout).trim().to_string();
            branch == "main" || branch == "master"
        }
        _ => false,
    }
}

/// Detect the base branch by checking if `main` or `master` exists.
fn detect_base_branch(root: &Path) -> Result<String> {
    for branch in &["main", "master"] {
        let output = process::Command::new("git")
            .args(["rev-parse", "--verify", branch])
            .current_dir(root)
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .status()?;
        if output.success() {
            return Ok(branch.to_string());
        }
    }
    bail!("Could not detect base branch: neither 'main' nor 'master' exists")
}

/// Changed line ranges of the working tree relative to HEAD: staged + unstaged
/// (one `git diff HEAD`), plus untracked files as whole-file changes.
pub(crate) fn working_tree_changes(root: &Path) -> Result<ChangedFiles> {
    let diff = git_diff_u0(root, &["HEAD"])?;
    let mut changed = canonicalize_ranges(root, parse_unified_diff(&diff));

    let output = process::Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        bail!("git ls-files failed");
    }
    for rel in String::from_utf8_lossy(&output.stdout).lines() {
        if rel.is_empty() {
            continue;
        }
        if let Ok(canonical) = root.join(rel).canonicalize() {
            changed.insert(canonical, ChangedSpan::WholeFile);
        }
    }

    Ok(changed)
}

/// Changed line ranges of the current branch relative to its merge-base with
/// the base branch. Returns the base branch name for display.
pub(crate) fn branch_changes(root: &Path) -> Result<(String, ChangedFiles)> {
    let base = detect_base_branch(root)?;

    let merge_base_output = process::Command::new("git")
        .args(["merge-base", "HEAD", &base])
        .current_dir(root)
        .output()?;
    if !merge_base_output.status.success() {
        bail!(
            "git merge-base failed — are you on a branch that shares history with '{}'?",
            base
        );
    }
    let merge_base =
        String::from_utf8_lossy(&merge_base_output.stdout).trim().to_string();

    let diff = git_diff_u0(root, &[&merge_base, "HEAD"])?;
    let changed = canonicalize_ranges(root, parse_unified_diff(&diff));
    Ok((base, changed))
}

/// Run `git diff -U0` with the given extra args and return raw output.
fn git_diff_u0(root: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = process::Command::new("git");
    cmd.arg("diff").arg("-U0");
    for arg in args {
        cmd.arg(arg);
    }
    let output = cmd.current_dir(root).output()?;
    if !output.status.success() {
        bail!("git diff failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Resolve relative diff paths to canonical absolute paths, dropping files
/// that no longer exist (a deleted file has no units to report on).
fn canonicalize_ranges(
    root: &Path,
    ranges: HashMap<String, Vec<(usize, usize)>>,
) -> ChangedFiles {
    let mut changed = ChangedFiles::new();
    for (rel, lines) in ranges {
        if let Ok(canonical) = root.join(&rel).canonicalize() {
            changed.insert(canonical, ChangedSpan::Lines(lines));
        }
    }
    changed
}

/// Parse unified diff output into new-side changed line ranges per file.
///
/// Each `@@ -a,b +c,d @@` hunk contributes the inclusive range `c..=c+d-1`.
/// A pure deletion (`d == 0`) has no new-side lines; it contributes the two
/// lines flanking the deletion point (`c` and `c+1`) so the unit the lines
/// were deleted from still counts as changed. Deleted files (`+++ /dev/null`)
/// are skipped.
fn parse_unified_diff(diff: &str) -> HashMap<String, Vec<(usize, usize)>> {
    let mut files: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    let mut current: Option<String> = None;

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            current = rest
                .strip_prefix("b/")
                .map(|p| p.to_string())
                .filter(|_| rest != "/dev/null");
            continue;
        }
        let Some(file) = current.as_ref() else { continue };
        let Some(rest) = line.strip_prefix("@@ ") else { continue };
        // "-a[,b] +c[,d] @@ ..."
        let Some(plus) = rest.split(' ').find(|s| s.starts_with('+')) else {
            continue;
        };
        let mut parts = plus[1..].split(',');
        let Some(Ok(start)) = parts.next().map(str::parse::<usize>) else {
            continue;
        };
        let count = match parts.next().map(str::parse::<usize>) {
            Some(Ok(c)) => c,
            Some(Err(_)) => continue,
            None => 1,
        };
        let range = if count == 0 {
            // Pure deletion: `start` is the line before the deletion point.
            (start.max(1), start + 1)
        } else {
            (start, start + count - 1)
        };
        files.entry(file.clone()).or_default().push(range);
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modification_hunks() {
        let diff = "\
diff --git a/src/foo.rs b/src/foo.rs
index 111..222 100644
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -10,2 +10,3 @@ fn foo() {
-old
+new
+new2
@@ -40 +41 @@ fn bar() {
-old
+new
";
        let files = parse_unified_diff(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files["src/foo.rs"], vec![(10, 12), (41, 41)]);
    }

    #[test]
    fn parses_multiple_files() {
        let diff = "\
--- a/src/a.rs
+++ b/src/a.rs
@@ -1 +1 @@
-x
+y
--- a/src/b.rs
+++ b/src/b.rs
@@ -5,0 +6,2 @@
+a
+b
";
        let files = parse_unified_diff(diff);
        assert_eq!(files["src/a.rs"], vec![(1, 1)]);
        assert_eq!(files["src/b.rs"], vec![(6, 7)]);
    }

    #[test]
    fn pure_deletion_flanks_deletion_point() {
        let diff = "\
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -12,3 +11,0 @@ fn foo() {
-a
-b
-c
";
        let files = parse_unified_diff(diff);
        assert_eq!(files["src/foo.rs"], vec![(11, 12)]);
    }

    #[test]
    fn deletion_at_top_of_file_clamps_to_line_one() {
        let diff = "\
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -1,2 +0,0 @@
-a
-b
";
        let files = parse_unified_diff(diff);
        assert_eq!(files["src/foo.rs"], vec![(1, 1)]);
    }

    #[test]
    fn deleted_file_is_skipped() {
        let diff = "\
--- a/src/gone.rs
+++ /dev/null
@@ -1,10 +0,0 @@
-stuff
";
        let files = parse_unified_diff(diff);
        assert!(files.is_empty());
    }

    #[test]
    fn new_file_collects_full_range() {
        let diff = "\
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1,20 @@
+stuff
";
        let files = parse_unified_diff(diff);
        assert_eq!(files["src/new.rs"], vec![(1, 20)]);
    }

    #[test]
    fn changed_span_overlap() {
        let span = ChangedSpan::Lines(vec![(10, 12), (40, 40)]);
        assert!(span.overlaps(1, 10)); // touches start of first range
        assert!(span.overlaps(12, 30)); // touches end of first range
        assert!(span.overlaps(35, 45)); // contains second range
        assert!(!span.overlaps(13, 39)); // between ranges
        assert!(!span.overlaps(1, 9)); // before
        assert!(ChangedSpan::WholeFile.overlaps(1, 1));
    }
}
