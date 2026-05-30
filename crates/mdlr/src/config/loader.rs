use super::types::{Config, METRIC_NAMES};
use anyhow::Result;
use std::fs;
use std::path::Path;

/// Load configuration from .mdlr/config.yaml at the given project root.
/// Returns default config if no file is found.
pub fn load_from_dir(root: &Path) -> Result<Config> {
    let config_path = root.join(".mdlr").join("config.yaml");
    if config_path.exists() {
        let contents = fs::read_to_string(&config_path)?;
        let config: Config = serde_yaml::from_str(&contents)?;
        warn_unknown_disabled_metrics(&config);
        Ok(config)
    } else {
        Ok(Config::default())
    }
}

/// Warn (and otherwise ignore) entries in `disabled_metrics` that don't name a
/// known metric — a typo leaves the metric enabled, so surface it on stderr.
fn warn_unknown_disabled_metrics(config: &Config) {
    for name in &config.disabled_metrics {
        if !METRIC_NAMES.contains(&name.as_str()) {
            eprintln!(
                "warning: unknown metric '{name}' in disabled_metrics (ignored). Run 'mdlr metrics ls' to see available metrics."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Write `.mdlr/config.yaml` under `root` with the given contents.
    fn write_config(root: &Path, yaml: &str) {
        let config_dir = root.join(".mdlr");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join("config.yaml"), yaml).unwrap();
    }

    #[test]
    fn test_load_default_when_no_config() {
        let temp = TempDir::new().unwrap();
        let config = load_from_dir(temp.path()).unwrap();
        assert_eq!(config.thresholds.dag_density.excellent, 0.5);
    }

    #[test]
    fn test_load_from_current_dir() {
        let temp = TempDir::new().unwrap();
        write_config(
            temp.path(),
            r#"
thresholds:
  dag_density:
    excellent: 0.3
    good: 0.8
    fair: 1.2
    poor: 1.8
"#,
        );

        let config = load_from_dir(temp.path()).unwrap();
        assert_eq!(config.thresholds.dag_density.excellent, 0.3);
        // Defaults still work for unspecified fields
        assert_eq!(config.thresholds.fan_in_max.excellent, 3.0);
    }

    #[test]
    fn test_load_disabled_metrics() {
        let temp = TempDir::new().unwrap();
        write_config(
            temp.path(),
            r#"
disabled_metrics:
  - lcom
  - duplication_pct
"#,
        );

        let config = load_from_dir(temp.path()).unwrap();
        assert!(config.is_disabled("lcom"));
        assert!(config.is_disabled("duplication_pct"));
        assert!(!config.is_disabled("cyclomatic"));
    }

    #[test]
    fn test_load_from_dir_does_not_search_parents() {
        let temp = TempDir::new().unwrap();
        write_config(
            temp.path(),
            r#"
thresholds:
  dag_density:
    excellent: 0.25
    good: 0.5
    fair: 0.75
    poor: 1.0
"#,
        );

        let child_dir = temp.path().join("child").join("grandchild");
        fs::create_dir_all(&child_dir).unwrap();

        // Should return default because child_dir has no .mdlr/config.yaml
        let config = load_from_dir(&child_dir).unwrap();
        assert_eq!(config.thresholds.dag_density.excellent, 0.5); // default
    }
}
