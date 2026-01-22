use crate::graph::Unit;
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
        Self {
            version: 1,
            files: HashMap::new(),
            last_scan: 0,
        }
    }
}
