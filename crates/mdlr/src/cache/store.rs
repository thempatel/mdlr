use super::ignores_store::IgnoresStore;
use anyhow::{Context, Result};
use mdlr_core::FileCacheEntry;
use std::fs;
use std::path::{Path, PathBuf};

const CACHE_DIR_NAME: &str = ".mdlr";
const CACHE_SUBDIR: &str = "cache";

/// Store for managing the .mdlr cache directory.
pub struct CacheStore {
    root: PathBuf,
}

impl CacheStore {
    /// Open or create a cache store at the given project root.
    pub fn open(root: &Path) -> Result<Self> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let cache_dir = root.join(CACHE_DIR_NAME).join(CACHE_SUBDIR);

        fs::create_dir_all(&cache_dir).with_context(|| {
            format!("Failed to create cache directory: {:?}", cache_dir)
        })?;

        Ok(Self { root })
    }

    /// Get the project root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the cache directory path (.mdlr/cache).
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join(CACHE_DIR_NAME).join(CACHE_SUBDIR)
    }

    /// Convert a source file path to its corresponding cache file path.
    /// e.g., src/foo.rs -> .mdlr/cache/src/foo.json
    pub fn cache_path(&self, source: &Path) -> PathBuf {
        let relative = source.strip_prefix(&self.root).unwrap_or(source);
        let mut cache_file = self.cache_dir().join(relative);
        cache_file.set_extension("json");
        cache_file
    }

    /// Load a cache entry for a source file.
    pub fn load_entry(&self, source: &Path) -> Result<Option<FileCacheEntry>> {
        let cache_path = self.cache_path(source);
        if !cache_path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&cache_path).with_context(|| {
            format!("Failed to read cache entry: {:?}", cache_path)
        })?;
        let entry: FileCacheEntry = serde_json::from_str(&content)
            .with_context(|| {
                format!("Failed to parse cache entry: {:?}", cache_path)
            })?;
        Ok(Some(entry))
    }

    /// Get an IgnoresStore for managing metric ignores.
    pub fn ignores(&self) -> IgnoresStore {
        IgnoresStore::new(self.root.join(CACHE_DIR_NAME))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_cache_path() {
        let temp = TempDir::new().unwrap();
        let store = CacheStore::open(temp.path()).unwrap();

        let source = temp.path().join("src/foo.rs");
        let cache = store.cache_path(&source);
        assert!(cache.ends_with("src/foo.json"));
    }
}
