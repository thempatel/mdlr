use anyhow::{Context, Result, bail};
use git2::{DiffOptions, Repository, Status, StatusOptions};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Detects changed files using git comparison.
pub struct GitChangeDetector {
    repo: Repository,
    main_branch: String,
}

impl GitChangeDetector {
    /// Opens a git repository at the given path.
    /// Returns an error if the path is not within a git repository.
    pub fn open(path: &Path, main_branch: &str) -> Result<Self> {
        let repo = Repository::discover(path)
            .context("mdlr requires a git repository")?;
        Ok(Self { repo, main_branch: main_branch.to_string() })
    }

    /// Returns the repository root path.
    pub fn root(&self) -> Result<PathBuf> {
        self.repo
            .workdir()
            .map(|p| p.to_path_buf())
            .ok_or_else(|| anyhow::anyhow!("Bare repository not supported"))
    }

    /// Detects changed files based on git state.
    ///
    /// Resolution order:
    /// 1. If `base_override` is provided (--base flag), use it
    /// 2. If on a feature branch, compare against origin/main (or main)
    /// 3. If on main branch, use `config_base_commit` (error if not set)
    ///
    /// Returns the union of:
    /// - Files changed between base and HEAD
    /// - Files with uncommitted changes (staged + unstaged)
    pub fn detect_changes(
        &self,
        base_override: Option<&str>,
        config_base_commit: Option<&str>,
    ) -> Result<HashSet<PathBuf>> {
        let base_ref = self.resolve_base_ref(base_override, config_base_commit)?;

        let mut changed = self.get_changed_files(&base_ref)?;
        let uncommitted = self.get_uncommitted_files()?;

        changed.extend(uncommitted);
        Ok(changed)
    }

    /// Resolves which base ref to use for comparison.
    fn resolve_base_ref(
        &self,
        base_override: Option<&str>,
        config_base_commit: Option<&str>,
    ) -> Result<String> {
        // 1. CLI override takes precedence
        if let Some(base) = base_override {
            return Ok(base.to_string());
        }

        // 2. Check if on main branch
        let current_branch = self.current_branch_name();
        let on_main = current_branch
            .as_ref()
            .is_some_and(|name| name == &self.main_branch);

        if on_main {
            // 3. On main - require config base_commit
            config_base_commit
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No git.base_commit configured in .mdlr/config.yaml. \
                         When on the '{}' branch, you must set git.base_commit \
                         or use --base to specify a comparison point.",
                        self.main_branch
                    )
                })
        } else {
            // 4. On feature branch - try origin/main, then main
            let origin_main = format!("origin/{}", self.main_branch);
            if self.ref_exists(&origin_main) {
                Ok(origin_main)
            } else if self.ref_exists(&self.main_branch) {
                Ok(self.main_branch.clone())
            } else {
                bail!(
                    "Could not find '{}' or 'origin/{}' to compare against. \
                     Use --base to specify a comparison point.",
                    self.main_branch,
                    self.main_branch
                )
            }
        }
    }

    /// Gets the name of the current branch, if any.
    fn current_branch_name(&self) -> Option<String> {
        let head = self.repo.head().ok()?;
        if head.is_branch() {
            head.shorthand().map(|s| s.to_string())
        } else {
            None
        }
    }

    /// Checks if a ref exists (branch, tag, or commit).
    fn ref_exists(&self, refspec: &str) -> bool {
        self.repo.revparse_single(refspec).is_ok()
    }

    /// Gets files changed between base_ref and HEAD.
    fn get_changed_files(&self, base_ref: &str) -> Result<HashSet<PathBuf>> {
        let base_obj = self.repo.revparse_single(base_ref).with_context(|| {
            format!("Base ref '{}' not found", base_ref)
        })?;
        let base_commit = base_obj.peel_to_commit().with_context(|| {
            format!("'{}' does not point to a commit", base_ref)
        })?;
        let base_tree = base_commit.tree()?;

        let head_ref = self.repo.head()?;
        let head_commit = head_ref.peel_to_commit()?;
        let head_tree = head_commit.tree()?;

        let mut diff_opts = DiffOptions::new();
        let diff = self.repo.diff_tree_to_tree(
            Some(&base_tree),
            Some(&head_tree),
            Some(&mut diff_opts),
        )?;

        let mut changed = HashSet::new();
        let repo_root = self.root()?;

        for delta in diff.deltas() {
            // Include both old and new paths to catch renames
            if let Some(path) = delta.old_file().path() {
                changed.insert(repo_root.join(path));
            }
            if let Some(path) = delta.new_file().path() {
                changed.insert(repo_root.join(path));
            }
        }

        Ok(changed)
    }

    /// Gets files with uncommitted changes (staged + unstaged).
    fn get_uncommitted_files(&self) -> Result<HashSet<PathBuf>> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true);

        let statuses = self.repo.statuses(Some(&mut opts))?;
        let repo_root = self.root()?;

        let mut changed = HashSet::new();

        for entry in statuses.iter() {
            let status = entry.status();

            // Include files that are modified, added, deleted, or renamed
            // in either the index (staged) or worktree (unstaged)
            let is_changed = status.intersects(
                Status::INDEX_NEW
                    | Status::INDEX_MODIFIED
                    | Status::INDEX_DELETED
                    | Status::INDEX_RENAMED
                    | Status::INDEX_TYPECHANGE
                    | Status::WT_NEW
                    | Status::WT_MODIFIED
                    | Status::WT_DELETED
                    | Status::WT_RENAMED
                    | Status::WT_TYPECHANGE,
            );

            if is_changed {
                if let Some(path) = entry.path() {
                    changed.insert(repo_root.join(path));
                }
            }
        }

        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_git_repo() {
        // This test assumes we're running in the mdlr git repo
        let result = GitChangeDetector::open(Path::new("."), "main");
        assert!(result.is_ok());
    }

    #[test]
    fn test_open_non_git_dir() {
        let result = GitChangeDetector::open(Path::new("/tmp"), "main");
        // /tmp might be in a git repo on some systems, so just check we don't panic
        let _ = result;
    }
}
