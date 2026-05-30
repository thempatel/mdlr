//! Git interaction for `mdlr check`'s diff mode: detecting the base branch and
//! collecting the set of files changed on the current branch / working tree.

use anyhow::{Result, bail};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process;

/// Check if the current HEAD is on the base branch (main or master).
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

/// Get staged and unstaged changes relative to HEAD.
/// Used when on main/master to check only the current working-tree modifications.
pub(crate) fn diff_files_head(root: &Path) -> Result<HashSet<PathBuf>> {
    let staged = git_diff_name_only(root, &["--cached"])?;
    let unstaged = git_diff_name_only(root, &[])?;

    let mut changed = HashSet::new();
    for rel in staged.iter().chain(unstaged.iter()) {
        let abs = root.join(rel);
        if let Ok(canonical) = abs.canonicalize() {
            changed.insert(canonical);
        }
    }

    Ok(changed)
}

/// Get the set of files changed on the current branch relative to its base.
/// Includes committed, staged, and unstaged changes (but not untracked files).
pub(crate) fn diff_files(root: &Path) -> Result<HashSet<PathBuf>> {
    let base = detect_base_branch(root)?;

    // Find merge base
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

    // Committed changes since merge base
    let committed = git_diff_name_only(root, &[&merge_base, "HEAD"])?;
    // Staged changes
    let staged = git_diff_name_only(root, &["--cached"])?;
    // Unstaged changes
    let unstaged = git_diff_name_only(root, &[])?;

    let mut changed = HashSet::new();
    for rel in committed.iter().chain(staged.iter()).chain(unstaged.iter()) {
        let abs = root.join(rel);
        if let Ok(canonical) = abs.canonicalize() {
            changed.insert(canonical);
        }
    }

    Ok(changed)
}

/// Run `git diff --name-only` with the given extra args and return the list of paths.
fn git_diff_name_only(root: &Path, args: &[&str]) -> Result<Vec<String>> {
    let mut cmd = process::Command::new("git");
    cmd.arg("diff").arg("--name-only");
    for arg in args {
        cmd.arg(arg);
    }
    let output = cmd.current_dir(root).output()?;
    if !output.status.success() {
        bail!("git diff --name-only failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect())
}
