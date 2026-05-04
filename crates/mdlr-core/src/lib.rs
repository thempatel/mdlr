pub mod graph;

pub use graph::{Edge, EdgeKind, Graph, Span, Unit, UnitKind, build};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Cached extraction data for a single source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCacheEntry {
    pub source_path: PathBuf,
    pub units: Vec<Unit>,
    pub cached_at: u64,
}
