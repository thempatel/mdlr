use crate::graph::Unit;
use crate::resolve::ResolutionContext;
use anyhow::Result;
use std::path::Path;

pub trait Extractor: Send + Sync {
    fn language(&self) -> &'static str;

    /// Extract units from source code.
    ///
    /// When a resolution context is provided, the extractor should:
    /// 1. Use crate-based IDs (e.g., "my_crate::module::func") instead of file-based
    /// 2. Resolve calls to their fully qualified crate paths
    ///
    /// The path should be the absolute path to the file for proper resolution.
    fn extract(
        &self,
        source: &str,
        path: &Path,
        resolution_ctx: Option<&ResolutionContext>,
    ) -> Result<Vec<Unit>>;
}
