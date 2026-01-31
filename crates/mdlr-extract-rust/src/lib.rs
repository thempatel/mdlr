mod extractor;
pub mod resolve;

pub use extractor::RustExtractor;
pub use mdlr_core::Extractor;
pub use resolve::{CargoWorkspace, ResolutionContext};

use std::path::Path;

/// Get an extractor for a file path based on its extension.
pub fn extractor_for_path(path: &Path) -> Option<Box<dyn Extractor>> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "rs" => Some(Box::new(RustExtractor::new().ok()?)),
        _ => None,
    }
}

/// Get all supported file extensions.
pub fn supported_extensions() -> &'static [&'static str] {
    &["rs"]
}
