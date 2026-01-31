//! Cargo.toml parsing for workspace and path dependency discovery.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Information about a Cargo workspace.
#[derive(Debug, Clone)]
pub struct CargoWorkspace {
    /// Root directory of the workspace (where the root Cargo.toml is).
    pub root: PathBuf,
    /// All crates in the workspace (members + path dependencies).
    pub members: Vec<CrateInfo>,
}

/// Information about a single crate.
#[derive(Debug, Clone)]
pub struct CrateInfo {
    /// Crate name (from [package] name).
    pub name: String,
    /// Root directory of the crate.
    pub root: PathBuf,
    /// Path to lib.rs (if this is a library crate).
    pub lib_path: Option<PathBuf>,
    /// Paths to binary entry points.
    pub bin_paths: Vec<PathBuf>,
    /// Path dependencies: (dependency_name, path_to_crate).
    pub path_deps: Vec<(String, PathBuf)>,
}

impl CargoWorkspace {
    /// Discover a Cargo workspace starting from a directory.
    ///
    /// Walks up to find the workspace root (Cargo.toml with [workspace]),
    /// then discovers all members and path dependencies.
    pub fn discover(start_dir: &Path) -> Result<Self> {
        let root_manifest = find_workspace_root(start_dir)?;
        let root = root_manifest
            .parent()
            .context("Cargo.toml has no parent directory")?
            .to_path_buf();

        let content =
            std::fs::read_to_string(&root_manifest).with_context(|| {
                format!("Failed to read {}", root_manifest.display())
            })?;
        let manifest: toml::Value = content.parse().with_context(|| {
            format!("Failed to parse {}", root_manifest.display())
        })?;

        let mut members = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Check if this is a workspace or a single crate
        if let Some(workspace) = manifest.get("workspace") {
            // Workspace manifest - discover members
            if let Some(member_patterns) =
                workspace.get("members").and_then(|v| v.as_array())
            {
                for pattern in member_patterns {
                    if let Some(pattern_str) = pattern.as_str() {
                        let member_paths =
                            expand_glob_pattern(&root, pattern_str);
                        for member_path in member_paths {
                            if seen.insert(member_path.clone()) {
                                if let Ok(crate_info) =
                                    parse_crate(&member_path)
                                {
                                    members.push(crate_info);
                                }
                            }
                        }
                    }
                }
            }

            // Also check if the root itself is a package
            if manifest.get("package").is_some() {
                if seen.insert(root.clone()) {
                    if let Ok(crate_info) = parse_crate(&root) {
                        members.push(crate_info);
                    }
                }
            }
        } else if manifest.get("package").is_some() {
            // Single crate (not a workspace)
            if let Ok(crate_info) = parse_crate(&root) {
                members.push(crate_info);
            }
        }

        // Recursively discover path dependencies
        let mut i = 0;
        while i < members.len() {
            let path_deps: Vec<_> = members[i].path_deps.clone();
            for (_, dep_path) in path_deps {
                if seen.insert(dep_path.clone()) {
                    if let Ok(crate_info) = parse_crate(&dep_path) {
                        members.push(crate_info);
                    }
                }
            }
            i += 1;
        }

        Ok(CargoWorkspace { root, members })
    }

    /// Find a crate by name.
    pub fn find_crate(&self, name: &str) -> Option<&CrateInfo> {
        self.members.iter().find(|c| c.name == name)
    }

    /// Get all crate names.
    pub fn crate_names(&self) -> Vec<&str> {
        self.members.iter().map(|c| c.name.as_str()).collect()
    }
}

impl CrateInfo {
    /// Get all entry point files for this crate (lib + bins).
    pub fn entry_points(&self) -> Vec<&Path> {
        let mut paths = Vec::new();
        if let Some(ref lib) = self.lib_path {
            paths.push(lib.as_path());
        }
        for bin in &self.bin_paths {
            paths.push(bin.as_path());
        }
        paths
    }
}

/// Find the workspace root Cargo.toml by walking up from start_dir.
fn find_workspace_root(start_dir: &Path) -> Result<PathBuf> {
    let start = if start_dir.is_file() {
        start_dir.parent().context("File has no parent directory")?
    } else {
        start_dir
    };

    let mut current = start.to_path_buf();

    // First, find any Cargo.toml
    let mut found_manifest = None;
    loop {
        let manifest_path = current.join("Cargo.toml");
        if manifest_path.exists() {
            found_manifest = Some(manifest_path);
            break;
        }
        if !current.pop() {
            break;
        }
    }

    let manifest_path =
        found_manifest.context("No Cargo.toml found in directory tree")?;

    // Check if this is a workspace root or if we need to go higher
    let content = std::fs::read_to_string(&manifest_path)?;
    let manifest: toml::Value = content.parse()?;

    // If it has [workspace], this is the root
    if manifest.get("workspace").is_some() {
        return Ok(manifest_path);
    }

    // Otherwise, check parent directories for a workspace root
    let mut current = manifest_path
        .parent()
        .context("Manifest has no parent")?
        .to_path_buf();

    while current.pop() {
        let parent_manifest = current.join("Cargo.toml");
        if parent_manifest.exists() {
            let content = std::fs::read_to_string(&parent_manifest)?;
            if let Ok(manifest) = content.parse::<toml::Value>() {
                if manifest.get("workspace").is_some() {
                    return Ok(parent_manifest);
                }
            }
        }
    }

    // No workspace found, the original manifest is the root
    Ok(manifest_path)
}

/// Expand a glob pattern relative to the workspace root.
fn expand_glob_pattern(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let mut results = Vec::new();

    // Handle simple patterns (no wildcards)
    if !pattern.contains('*') {
        let path = root.join(pattern);
        if path.join("Cargo.toml").exists() {
            results.push(path);
        }
        return results;
    }

    // Handle patterns like "crates/*"
    if pattern.ends_with("/*") {
        let prefix = &pattern[..pattern.len() - 2];
        let dir = root.join(prefix);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join("Cargo.toml").exists() {
                    results.push(path);
                }
            }
        }
        return results;
    }

    // Handle more complex patterns - fall back to simple matching
    // For now, just try the literal path
    let path = root.join(pattern.replace('*', ""));
    if path.join("Cargo.toml").exists() {
        results.push(path);
    }

    results
}

/// Parse a single crate's Cargo.toml.
fn parse_crate(crate_root: &Path) -> Result<CrateInfo> {
    let manifest_path = crate_root.join("Cargo.toml");
    let content =
        std::fs::read_to_string(&manifest_path).with_context(|| {
            format!("Failed to read {}", manifest_path.display())
        })?;
    let manifest: toml::Value = content.parse().with_context(|| {
        format!("Failed to parse {}", manifest_path.display())
    })?;

    // Get package name
    let name = manifest
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .context("Missing [package] name")?
        .to_string();

    // Get lib path
    let lib_path = get_lib_path(&manifest, crate_root);

    // Get bin paths
    let bin_paths = get_bin_paths(&manifest, crate_root);

    // Get path dependencies
    let path_deps = get_path_dependencies(&manifest, crate_root);

    Ok(CrateInfo {
        name,
        root: crate_root.to_path_buf(),
        lib_path,
        bin_paths,
        path_deps,
    })
}

/// Get the library entry point path.
fn get_lib_path(manifest: &toml::Value, crate_root: &Path) -> Option<PathBuf> {
    // Check for explicit [lib] path
    if let Some(lib) = manifest.get("lib") {
        if let Some(path) = lib.get("path").and_then(|p| p.as_str()) {
            let lib_path = crate_root.join(path);
            if lib_path.exists() {
                return Some(lib_path);
            }
        }
    }

    // Default: src/lib.rs
    let default_lib = crate_root.join("src/lib.rs");
    if default_lib.exists() {
        return Some(default_lib);
    }

    None
}

/// Get binary entry point paths.
fn get_bin_paths(manifest: &toml::Value, crate_root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Check for explicit [[bin]] entries
    if let Some(bins) = manifest.get("bin").and_then(|b| b.as_array()) {
        for bin in bins {
            if let Some(path) = bin.get("path").and_then(|p| p.as_str()) {
                let bin_path = crate_root.join(path);
                if bin_path.exists() {
                    paths.push(bin_path);
                }
            }
        }
    }

    // If no explicit bins, check defaults
    if paths.is_empty() {
        // Default: src/main.rs
        let main_rs = crate_root.join("src/main.rs");
        if main_rs.exists() {
            paths.push(main_rs);
        }

        // Also check src/bin/*.rs
        let bin_dir = crate_root.join("src/bin");
        if let Ok(entries) = std::fs::read_dir(&bin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "rs") {
                    paths.push(path);
                }
            }
        }
    }

    paths
}

/// Extract path dependencies from Cargo.toml.
fn get_path_dependencies(
    manifest: &toml::Value,
    crate_root: &Path,
) -> Vec<(String, PathBuf)> {
    let mut deps = Vec::new();

    // Check [dependencies], [dev-dependencies], [build-dependencies]
    for section in &["dependencies", "dev-dependencies", "build-dependencies"]
    {
        if let Some(deps_table) =
            manifest.get(section).and_then(|d| d.as_table())
        {
            for (name, value) in deps_table {
                if let Some(path) = extract_path_from_dep(value) {
                    let full_path = crate_root.join(path);
                    if full_path.exists() {
                        deps.push((name.clone(), full_path));
                    }
                }
            }
        }
    }

    // Check [target.'cfg(...)'.dependencies]
    if let Some(targets) = manifest.get("target").and_then(|t| t.as_table()) {
        for (_target_cfg, target_manifest) in targets {
            for section in
                &["dependencies", "dev-dependencies", "build-dependencies"]
            {
                if let Some(deps_table) =
                    target_manifest.get(section).and_then(|d| d.as_table())
                {
                    for (name, value) in deps_table {
                        if let Some(path) = extract_path_from_dep(value) {
                            let full_path = crate_root.join(path);
                            if full_path.exists() {
                                deps.push((name.clone(), full_path));
                            }
                        }
                    }
                }
            }
        }
    }

    deps
}

/// Extract path from a dependency value (handles both inline and table formats).
fn extract_path_from_dep(value: &toml::Value) -> Option<&str> {
    match value {
        toml::Value::Table(t) => t.get("path").and_then(|p| p.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_cargo_toml(dir: &Path, content: &str) {
        fs::write(dir.join("Cargo.toml"), content).unwrap();
    }

    fn create_src_file(dir: &Path, filename: &str) {
        let src_dir = dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join(filename), "// placeholder").unwrap();
    }

    #[test]
    fn test_single_crate() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_cargo_toml(
            root,
            r#"
[package]
name = "my-crate"
version = "0.1.0"
edition = "2021"
"#,
        );
        create_src_file(root, "lib.rs");

        let workspace = CargoWorkspace::discover(root).unwrap();
        assert_eq!(workspace.members.len(), 1);
        assert_eq!(workspace.members[0].name, "my-crate");
        assert!(workspace.members[0].lib_path.is_some());
    }

    #[test]
    fn test_workspace_with_members() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Root Cargo.toml
        create_cargo_toml(
            root,
            r#"
[workspace]
members = ["crate-a", "crate-b"]
"#,
        );

        // crate-a
        let crate_a = root.join("crate-a");
        fs::create_dir_all(&crate_a).unwrap();
        create_cargo_toml(
            &crate_a,
            r#"
[package]
name = "crate-a"
version = "0.1.0"
"#,
        );
        create_src_file(&crate_a, "lib.rs");

        // crate-b
        let crate_b = root.join("crate-b");
        fs::create_dir_all(&crate_b).unwrap();
        create_cargo_toml(
            &crate_b,
            r#"
[package]
name = "crate-b"
version = "0.1.0"
"#,
        );
        create_src_file(&crate_b, "lib.rs");

        let workspace = CargoWorkspace::discover(root).unwrap();
        assert_eq!(workspace.members.len(), 2);

        let names: Vec<_> =
            workspace.members.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"crate-a"));
        assert!(names.contains(&"crate-b"));
    }

    #[test]
    fn test_path_dependencies() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Main crate
        create_cargo_toml(
            root,
            r#"
[package]
name = "main-crate"
version = "0.1.0"

[dependencies]
local-dep = { path = "./local-dep" }
"#,
        );
        create_src_file(root, "main.rs");

        // Local dependency
        let dep_dir = root.join("local-dep");
        fs::create_dir_all(&dep_dir).unwrap();
        create_cargo_toml(
            &dep_dir,
            r#"
[package]
name = "local-dep"
version = "0.1.0"
"#,
        );
        create_src_file(&dep_dir, "lib.rs");

        let workspace = CargoWorkspace::discover(root).unwrap();

        // Should discover both the main crate and the path dependency
        assert_eq!(workspace.members.len(), 2);

        let main_crate = workspace.find_crate("main-crate").unwrap();
        assert_eq!(main_crate.path_deps.len(), 1);
        assert_eq!(main_crate.path_deps[0].0, "local-dep");
    }

    #[test]
    fn test_custom_lib_path() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_cargo_toml(
            root,
            r#"
[package]
name = "custom-lib"
version = "0.1.0"

[lib]
path = "src/my_lib.rs"
"#,
        );
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("my_lib.rs"), "// custom lib").unwrap();

        let workspace = CargoWorkspace::discover(root).unwrap();
        let crate_info = &workspace.members[0];

        assert!(crate_info.lib_path.is_some());
        assert!(crate_info.lib_path.as_ref().unwrap().ends_with("my_lib.rs"));
    }

    #[test]
    fn test_glob_workspace_members() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Root Cargo.toml with glob pattern
        create_cargo_toml(
            root,
            r#"
[workspace]
members = ["crates/*"]
"#,
        );

        // Create crates directory with multiple crates
        let crates_dir = root.join("crates");
        fs::create_dir_all(&crates_dir).unwrap();

        for name in &["foo", "bar", "baz"] {
            let crate_dir = crates_dir.join(name);
            fs::create_dir_all(&crate_dir).unwrap();
            create_cargo_toml(
                &crate_dir,
                &format!(
                    r#"
[package]
name = "{name}"
version = "0.1.0"
"#
                ),
            );
            create_src_file(&crate_dir, "lib.rs");
        }

        let workspace = CargoWorkspace::discover(root).unwrap();
        assert_eq!(workspace.members.len(), 3);

        let names: Vec<_> =
            workspace.members.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"bar"));
        assert!(names.contains(&"baz"));
    }
}
