use crate::extract::supported_extensions;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

/// Walker for traversing source files in a project, respecting .gitignore.
pub struct SourceWalker {
    root: PathBuf,
}

impl SourceWalker {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Walk the source tree, yielding paths to supported source files.
    /// Respects .gitignore and other standard ignore patterns.
    pub fn walk(&self) -> impl Iterator<Item = PathBuf> {
        let extensions = supported_extensions();

        WalkBuilder::new(&self.root)
            .standard_filters(true)
            .hidden(true)
            .build()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_type()
                    .map(|ft| ft.is_file())
                    .unwrap_or(false)
            })
            .filter(move |entry| has_supported_extension(entry.path(), extensions))
            .map(|entry| entry.into_path())
    }
}

fn has_supported_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| extensions.contains(&ext))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_walk_finds_rust_files() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();

        fs::write(src_dir.join("main.rs"), "fn main() {}").unwrap();
        fs::write(src_dir.join("lib.rs"), "pub mod foo;").unwrap();
        fs::write(src_dir.join("readme.txt"), "not code").unwrap();

        let walker = SourceWalker::new(temp.path());
        let files: Vec<_> = walker.walk().collect();

        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|p| p.ends_with("main.rs")));
        assert!(files.iter().any(|p| p.ends_with("lib.rs")));
    }

    #[test]
    fn test_walk_respects_gitignore() {
        use std::process::Command;

        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("src");
        let target_dir = temp.path().join("target");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&target_dir).unwrap();

        // Initialize git repo so .gitignore is recognized
        Command::new("git")
            .args(["init"])
            .current_dir(temp.path())
            .output()
            .expect("git init failed");

        fs::write(temp.path().join(".gitignore"), "target/\n").unwrap();
        fs::write(src_dir.join("main.rs"), "fn main() {}").unwrap();
        fs::write(target_dir.join("debug.rs"), "fn debug() {}").unwrap();

        let walker = SourceWalker::new(temp.path());
        let files: Vec<_> = walker.walk().collect();

        assert_eq!(files.len(), 1);
        assert!(files.iter().any(|p| p.ends_with("main.rs")));
        assert!(!files.iter().any(|p| p.to_string_lossy().contains("target")));
    }
}
