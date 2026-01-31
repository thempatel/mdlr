use super::types::{FileCacheEntry, FileMetadata, ProjectIndex};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_DIR_NAME: &str = ".mdlr";
const CACHE_SUBDIR: &str = "cache";
const INDEX_FILE: &str = "index.json";
const TAGS_FILE: &str = "tags.json";
const STAGED_TAGS_FILE: &str = "tags.staged.json";

/// Store for managing the .mdlr cache directory.
pub struct CacheStore {
    root: PathBuf,
    cache_dir: PathBuf,
    index_path: PathBuf,
    pub(super) tags_path: PathBuf,
    pub(super) staged_tags_path: PathBuf,
}

impl CacheStore {
    /// Open or create a cache store at the given project root.
    pub fn open(root: &Path) -> Result<Self> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mdlr_dir = root.join(CACHE_DIR_NAME);
        let cache_dir = mdlr_dir.join(CACHE_SUBDIR);
        let index_path = mdlr_dir.join(INDEX_FILE);
        let tags_path = mdlr_dir.join(TAGS_FILE);
        let staged_tags_path = mdlr_dir.join(STAGED_TAGS_FILE);

        fs::create_dir_all(&cache_dir).with_context(|| {
            format!("Failed to create cache directory: {:?}", cache_dir)
        })?;

        Ok(Self { root, cache_dir, index_path, tags_path, staged_tags_path })
    }

    /// Find and open a cache store by searching up from the given directory.
    /// Returns an error if no .mdlr directory is found.
    pub fn find(start_dir: &Path) -> Result<Self> {
        let start = start_dir
            .canonicalize()
            .unwrap_or_else(|_| start_dir.to_path_buf());
        let mut current = start.as_path();

        loop {
            let mdlr_dir = current.join(CACHE_DIR_NAME);
            if mdlr_dir.exists() && mdlr_dir.is_dir() {
                return Self::open(current);
            }

            match current.parent() {
                Some(parent) => current = parent,
                None => anyhow::bail!(
                    "No .mdlr directory found. Run 'mdlr check --save' to initialize."
                ),
            }
        }
    }

    /// Find a cache store by searching up, or create one at the given directory if not found.
    pub fn find_or_create(start_dir: &Path) -> Result<Self> {
        let start = start_dir
            .canonicalize()
            .unwrap_or_else(|_| start_dir.to_path_buf());
        let mut current = start.as_path();

        loop {
            let mdlr_dir = current.join(CACHE_DIR_NAME);
            if mdlr_dir.exists() && mdlr_dir.is_dir() {
                return Self::open(current);
            }

            match current.parent() {
                Some(parent) => current = parent,
                None => {
                    // No existing .mdlr found, create at start_dir
                    return Self::open(start_dir);
                }
            }
        }
    }

    /// Get the project root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Convert a source file path to its corresponding cache file path.
    /// e.g., src/foo.rs -> .mdlr/cache/src/foo.json
    pub fn cache_path(&self, source: &Path) -> PathBuf {
        let relative = source.strip_prefix(&self.root).unwrap_or(source);
        let mut cache_file = self.cache_dir.join(relative);
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

    /// Save a cache entry for a source file.
    pub fn save_entry(&self, entry: &FileCacheEntry) -> Result<()> {
        let cache_path = self.cache_path(&entry.source_path);

        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create cache directory: {:?}", parent)
            })?;
        }

        let content = serde_json::to_string_pretty(entry)?;
        fs::write(&cache_path, content).with_context(|| {
            format!("Failed to write cache entry: {:?}", cache_path)
        })?;
        Ok(())
    }

    /// Load the project index.
    pub fn load_index(&self) -> Result<ProjectIndex> {
        if !self.index_path.exists() {
            return Ok(ProjectIndex::default());
        }

        let content =
            fs::read_to_string(&self.index_path).with_context(|| {
                format!("Failed to read index: {:?}", self.index_path)
            })?;
        let index: ProjectIndex = serde_json::from_str(&content)
            .with_context(|| {
                format!("Failed to parse index: {:?}", self.index_path)
            })?;
        Ok(index)
    }

    /// Save the project index.
    pub fn save_index(&self, index: &ProjectIndex) -> Result<()> {
        let content = serde_json::to_string_pretty(index)?;
        fs::write(&self.index_path, content).with_context(|| {
            format!("Failed to write index: {:?}", self.index_path)
        })?;
        Ok(())
    }

    /// Check if a source file is stale (needs re-extraction).
    /// Returns true if the file is new or has changed since last cache.
    pub fn is_stale(&self, source: &Path) -> Result<bool> {
        let cache_entry = self.load_entry(source)?;
        match cache_entry {
            None => Ok(true),
            Some(entry) => {
                let current = get_file_metadata(source)?;
                Ok(entry.mtime != current.mtime || entry.size != current.size)
            }
        }
    }
}

/// Get the mtime and size of a file.
pub fn get_file_metadata(path: &Path) -> Result<FileMetadata> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("Failed to get metadata for: {:?}", path))?;

    let mtime = metadata
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let size = metadata.len();

    Ok(FileMetadata { mtime, size })
}

/// Get current timestamp as seconds since UNIX epoch.
pub fn now_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
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

    #[test]
    fn test_index_roundtrip() {
        let temp = TempDir::new().unwrap();
        let store = CacheStore::open(temp.path()).unwrap();

        let mut index = ProjectIndex::default();
        index.files.insert(
            PathBuf::from("src/main.rs"),
            FileMetadata { mtime: 123, size: 456 },
        );

        store.save_index(&index).unwrap();
        let loaded = store.load_index().unwrap();

        assert_eq!(loaded.files.len(), 1);
        assert_eq!(
            loaded.files.get(&PathBuf::from("src/main.rs")).unwrap().mtime,
            123
        );
    }
}
