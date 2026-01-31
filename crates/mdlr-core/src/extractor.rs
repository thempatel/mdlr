use crate::graph::Unit;
use anyhow::Result;
use std::path::Path;

/// Language-agnostic trait for extracting units from source code.
///
/// Each language implementation (Rust, Python, etc.) provides its own
/// implementation of this trait.
pub trait Extractor: Send + Sync {
    /// The name of the language this extractor handles.
    fn language(&self) -> &'static str;

    /// File extensions this extractor handles (e.g., &["rs"] for Rust).
    fn extensions(&self) -> &'static [&'static str];

    /// Extract units from source code.
    ///
    /// # Arguments
    /// * `source` - The source code to extract units from
    /// * `path` - The path to the source file (used for generating IDs)
    ///
    /// # Returns
    /// A vector of units extracted from the source code.
    fn extract(&self, source: &str, path: &Path) -> Result<Vec<Unit>>;
}
