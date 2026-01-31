use anyhow::{Result, bail};
use mdlr_core::Unit;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Metadata for a single source file used for staleness detection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileMetadata {
    pub mtime: u64,
    pub size: u64,
}

/// Cached extraction data for a single source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCacheEntry {
    pub source_path: PathBuf,
    pub mtime: u64,
    pub size: u64,
    pub units: Vec<Unit>,
    pub cached_at: u64,
}

/// Project-wide index tracking all known files and their metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectIndex {
    pub version: u32,
    pub files: HashMap<PathBuf, FileMetadata>,
    pub last_scan: u64,
}

impl Default for ProjectIndex {
    fn default() -> Self {
        Self { version: 1, files: HashMap::new(), last_scan: 0 }
    }
}

/// User-defined semantic tags stored separately from extracted units.
/// This allows tags to persist across re-extraction when source files change.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SemanticTags {
    pub version: u32,
    pub tags: HashMap<String, Vec<String>>, // unit_id -> tags
}

impl SemanticTags {
    pub fn new() -> Self {
        Self { version: 1, tags: HashMap::new() }
    }

    /// Add a tag to a unit. Validates the tag format (namespace:value).
    pub fn add_tag(&mut self, unit_id: &str, tag: &str) -> Result<()> {
        validate_tag(tag)?;
        let entry = self.tags.entry(unit_id.to_string()).or_default();
        if !entry.contains(&tag.to_string()) {
            entry.push(tag.to_string());
        }
        Ok(())
    }

    /// Remove a tag from a unit.
    pub fn remove_tag(&mut self, unit_id: &str, tag: &str) -> bool {
        if let Some(entry) = self.tags.get_mut(unit_id) {
            if let Some(pos) = entry.iter().position(|t| t == tag) {
                entry.remove(pos);
                if entry.is_empty() {
                    self.tags.remove(unit_id);
                }
                return true;
            }
        }
        false
    }

    /// Clear all tags from a unit.
    pub fn clear_tags(&mut self, unit_id: &str) -> bool {
        self.tags.remove(unit_id).is_some()
    }

    /// Get tags for a unit.
    pub fn get_tags(&self, unit_id: &str) -> &[String] {
        self.tags.get(unit_id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Merge staged changes into this tag set.
    pub fn merge_staged(&mut self, staged: &StagedTags) {
        // Apply additions
        for (unit_id, tags_to_add) in &staged.additions {
            let entry = self.tags.entry(unit_id.clone()).or_default();
            for tag in tags_to_add {
                if !entry.contains(tag) {
                    entry.push(tag.clone());
                }
            }
        }

        // Apply removals
        for (unit_id, tags_to_remove) in &staged.removals {
            if let Some(entry) = self.tags.get_mut(unit_id) {
                entry.retain(|t| !tags_to_remove.contains(t));
                if entry.is_empty() {
                    self.tags.remove(unit_id);
                }
            }
        }

        // Apply clears
        for unit_id in &staged.clears {
            self.tags.remove(unit_id);
        }
    }

    /// Create a view with staged changes overlaid (without modifying self).
    pub fn with_staged(&self, staged: &StagedTags) -> SemanticTags {
        let mut merged = self.clone();
        merged.merge_staged(staged);
        merged
    }
}

/// Staged tag changes that haven't been committed yet.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StagedTags {
    pub version: u32,
    pub additions: HashMap<String, Vec<String>>, // unit_id -> tags to add
    pub removals: HashMap<String, Vec<String>>,  // unit_id -> tags to remove
    pub clears: Vec<String>, // unit_ids to clear all tags from
}

impl StagedTags {
    pub fn new() -> Self {
        Self {
            version: 1,
            additions: HashMap::new(),
            removals: HashMap::new(),
            clears: Vec::new(),
        }
    }

    /// Check if there are any staged changes.
    pub fn is_empty(&self) -> bool {
        self.additions.is_empty()
            && self.removals.is_empty()
            && self.clears.is_empty()
    }

    /// Stage a tag addition.
    pub fn stage_add(&mut self, unit_id: &str, tag: &str) -> Result<()> {
        validate_tag(tag)?;

        // If this unit was cleared, remove it from clears since we're adding to it
        self.clears.retain(|id| id != unit_id);

        // Remove from removals if present
        if let Some(removals) = self.removals.get_mut(unit_id) {
            removals.retain(|t| t != tag);
            if removals.is_empty() {
                self.removals.remove(unit_id);
            }
        }

        // Add to additions
        let entry = self.additions.entry(unit_id.to_string()).or_default();
        if !entry.contains(&tag.to_string()) {
            entry.push(tag.to_string());
        }
        Ok(())
    }

    /// Stage a tag removal.
    pub fn stage_remove(&mut self, unit_id: &str, tag: &str) {
        // Remove from additions if present
        if let Some(additions) = self.additions.get_mut(unit_id) {
            additions.retain(|t| t != tag);
            if additions.is_empty() {
                self.additions.remove(unit_id);
            }
        }

        // Add to removals
        let entry = self.removals.entry(unit_id.to_string()).or_default();
        if !entry.contains(&tag.to_string()) {
            entry.push(tag.to_string());
        }
    }

    /// Stage clearing all tags from a unit.
    pub fn stage_clear(&mut self, unit_id: &str) {
        // Remove any additions for this unit
        self.additions.remove(unit_id);
        // Remove any specific removals for this unit
        self.removals.remove(unit_id);
        // Add to clears if not already present
        if !self.clears.contains(&unit_id.to_string()) {
            self.clears.push(unit_id.to_string());
        }
    }
}

/// Validate that a tag has the format namespace:value.
pub fn validate_tag(tag: &str) -> Result<()> {
    let parts: Vec<&str> = tag.split(':').collect();
    if parts.len() != 2 {
        bail!(
            "Invalid tag format '{}': must be 'namespace:value' (exactly one colon)",
            tag
        );
    }
    if parts[0].is_empty() || parts[1].is_empty() {
        bail!(
            "Invalid tag format '{}': namespace and value must not be empty",
            tag
        );
    }
    Ok(())
}
