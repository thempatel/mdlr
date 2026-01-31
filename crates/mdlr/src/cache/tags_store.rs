//! Tag storage operations for CacheStore.
//!
//! This module extends CacheStore with methods for managing semantic tags
//! and staged tag changes.

use super::store::CacheStore;
use super::types::{SemanticTags, StagedTags};
use anyhow::{Context, Result};
use std::fs;

/// Tag storage operations for CacheStore.
impl CacheStore {
    /// Load semantic tags.
    pub fn load_tags(&self) -> Result<SemanticTags> {
        if !self.tags_path.exists() {
            return Ok(SemanticTags::new());
        }

        let content =
            fs::read_to_string(&self.tags_path).with_context(|| {
                format!("Failed to read tags: {:?}", self.tags_path)
            })?;
        let tags: SemanticTags =
            serde_json::from_str(&content).with_context(|| {
                format!("Failed to parse tags: {:?}", self.tags_path)
            })?;
        Ok(tags)
    }

    /// Save semantic tags.
    pub fn save_tags(&self, tags: &SemanticTags) -> Result<()> {
        let content = serde_json::to_string_pretty(tags)?;
        fs::write(&self.tags_path, content).with_context(|| {
            format!("Failed to write tags: {:?}", self.tags_path)
        })?;
        Ok(())
    }

    /// Load staged tag changes.
    pub fn load_staged_tags(&self) -> Result<StagedTags> {
        if !self.staged_tags_path.exists() {
            return Ok(StagedTags::new());
        }

        let content = fs::read_to_string(&self.staged_tags_path)
            .with_context(|| {
                format!(
                    "Failed to read staged tags: {:?}",
                    self.staged_tags_path
                )
            })?;
        let staged: StagedTags =
            serde_json::from_str(&content).with_context(|| {
                format!(
                    "Failed to parse staged tags: {:?}",
                    self.staged_tags_path
                )
            })?;
        Ok(staged)
    }

    /// Save staged tag changes.
    pub fn save_staged_tags(&self, staged: &StagedTags) -> Result<()> {
        if staged.is_empty() {
            // Remove the file if there are no staged changes
            if self.staged_tags_path.exists() {
                fs::remove_file(&self.staged_tags_path).with_context(
                    || {
                        format!(
                            "Failed to remove staged tags: {:?}",
                            self.staged_tags_path
                        )
                    },
                )?;
            }
            return Ok(());
        }

        let content = serde_json::to_string_pretty(staged)?;
        fs::write(&self.staged_tags_path, content).with_context(|| {
            format!("Failed to write staged tags: {:?}", self.staged_tags_path)
        })?;
        Ok(())
    }

    /// Check if there are staged tag changes.
    pub fn has_staged_tags(&self) -> bool {
        self.staged_tags_path.exists()
    }

    /// Commit staged tags: merge into main tags and remove staged file.
    pub fn commit_staged_tags(&self) -> Result<bool> {
        let staged = self.load_staged_tags()?;
        if staged.is_empty() {
            return Ok(false);
        }

        let mut tags = self.load_tags()?;
        tags.merge_staged(&staged);
        self.save_tags(&tags)?;

        // Remove staged file
        if self.staged_tags_path.exists() {
            fs::remove_file(&self.staged_tags_path).with_context(|| {
                format!(
                    "Failed to remove staged tags: {:?}",
                    self.staged_tags_path
                )
            })?;
        }

        Ok(true)
    }

    /// Load tags with staged changes overlaid (for reading).
    pub fn load_tags_with_staged(&self) -> Result<SemanticTags> {
        let tags = self.load_tags()?;
        let staged = self.load_staged_tags()?;
        Ok(tags.with_staged(&staged))
    }
}
