use super::types::Config;
use anyhow::Result;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const CONFIG_DIR: &str = ".mdlr";
const CONFIG_FILE: &str = "config.yaml";

/// Find config file by searching recursively up from the given directory
fn find_config_file(start_dir: &Path) -> Option<PathBuf> {
    let mut current = start_dir.to_path_buf();

    loop {
        let config_path = current.join(CONFIG_DIR).join(CONFIG_FILE);
        if config_path.exists() {
            return Some(config_path);
        }

        if !current.pop() {
            return None;
        }
    }
}

/// Load configuration from .mdlr/config.yaml
/// Searches recursively up from the current working directory
/// Returns default config if no file is found
pub fn load() -> Result<Config> {
    load_from_dir(&env::current_dir()?)
}

/// Load configuration starting from a specific directory
pub fn load_from_dir(start_dir: &Path) -> Result<Config> {
    match find_config_file(start_dir) {
        Some(path) => {
            let contents = fs::read_to_string(&path)?;
            let config: Config = serde_yaml::from_str(&contents)?;
            Ok(config)
        }
        None => Ok(Config::default()),
    }
}

/// Returns the path where config was found, if any
pub fn find_config_path() -> Option<PathBuf> {
    env::current_dir().ok().and_then(|dir| find_config_file(&dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_load_default_when_no_config() {
        let temp = TempDir::new().unwrap();
        let config = load_from_dir(temp.path()).unwrap();
        assert_eq!(config.thresholds.dag_density.excellent, 0.5);
    }

    #[test]
    fn test_load_from_current_dir() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".mdlr");
        fs::create_dir(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.yaml"),
            r#"
thresholds:
  dag_density:
    excellent: 0.3
    good: 0.8
    fair: 1.2
    poor: 1.8
"#,
        )
        .unwrap();

        let config = load_from_dir(temp.path()).unwrap();
        assert_eq!(config.thresholds.dag_density.excellent, 0.3);
        // Defaults still work for unspecified fields
        assert_eq!(config.thresholds.fan_in_max.excellent, 3.0);
    }

    #[test]
    fn test_load_from_parent_dir() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".mdlr");
        fs::create_dir(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.yaml"),
            r#"
thresholds:
  dag_density:
    excellent: 0.25
    good: 0.5
    fair: 0.75
    poor: 1.0
"#,
        )
        .unwrap();

        let child_dir = temp.path().join("child").join("grandchild");
        fs::create_dir_all(&child_dir).unwrap();

        let config = load_from_dir(&child_dir).unwrap();
        assert_eq!(config.thresholds.dag_density.excellent, 0.25);
    }

    #[test]
    fn test_find_config_path() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".mdlr");
        fs::create_dir(&config_dir).unwrap();
        let config_path = config_dir.join("config.yaml");
        fs::write(&config_path, "thresholds: {}").unwrap();

        let found = find_config_file(temp.path());
        assert!(found.is_some());
        assert_eq!(found.unwrap(), config_path);
    }
}
