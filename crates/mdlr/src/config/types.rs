use serde::{Deserialize, Serialize};

/// Quality bucket for metric values
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bucket {
    Excellent,
    Good,
    Fair,
    Poor,
    Critical,
}

impl std::fmt::Display for Bucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Bucket::Excellent => write!(f, "excellent"),
            Bucket::Good => write!(f, "good"),
            Bucket::Fair => write!(f, "fair"),
            Bucket::Poor => write!(f, "poor"),
            Bucket::Critical => write!(f, "critical"),
        }
    }
}

/// Display mode for metric output
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    #[default]
    Both,
    Label,
    Value,
}

/// Threshold configuration for a single metric
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricThresholds {
    pub excellent: f64,
    pub good: f64,
    pub fair: f64,
    pub poor: f64,
}

impl MetricThresholds {
    /// Evaluate a higher-is-worse metric. Field names match the bucket the
    /// value falls into when below that threshold.
    pub fn evaluate(&self, value: f64) -> Bucket {
        if value < self.excellent {
            Bucket::Excellent
        } else if value < self.good {
            Bucket::Good
        } else if value < self.fair {
            Bucket::Fair
        } else if value < self.poor {
            Bucket::Poor
        } else {
            Bucket::Critical
        }
    }

    /// Evaluate a lower-is-worse metric (e.g. coverage %). Fields name the
    /// LOW boundary of each bucket; a value at-or-above the field is in
    /// that bucket or better.
    pub fn evaluate_asc(&self, value: f64) -> Bucket {
        if value >= self.excellent {
            Bucket::Excellent
        } else if value >= self.good {
            Bucket::Good
        } else if value >= self.fair {
            Bucket::Fair
        } else if value >= self.poor {
            Bucket::Poor
        } else {
            Bucket::Critical
        }
    }
}

/// Display configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    #[serde(default)]
    pub mode: DisplayMode,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self { mode: DisplayMode::Both }
    }
}

/// All thresholds configuration. Container-level `#[serde(default)]` fills any
/// missing field from the `Default` impl below, so a config may override just
/// the metrics it cares about.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThresholdsConfig {
    pub dag_density: MetricThresholds,
    pub fan_in_max: MetricThresholds,
    pub fan_in_mean: MetricThresholds,
    pub fan_out_max: MetricThresholds,
    pub fan_out_mean: MetricThresholds,
    pub function_size: MetricThresholds,
    pub params: MetricThresholds,
    pub cyclomatic: MetricThresholds,
    pub cognitive: MetricThresholds,
    pub methods_per_struct: MetricThresholds,
    pub lcom: MetricThresholds,
    pub file_loc: MetricThresholds,
    pub max_scope: MetricThresholds,
    pub duplication_pct: MetricThresholds,
    pub line_cov: MetricThresholds,
    pub uncov_branches: MetricThresholds,
}

/// Default threshold values as constants
mod defaults {
    use super::MetricThresholds;

    pub const DAG_DENSITY: MetricThresholds =
        MetricThresholds { excellent: 0.5, good: 1.0, fair: 1.5, poor: 2.0 };

    pub const FAN_IN_MAX: MetricThresholds =
        MetricThresholds { excellent: 3.0, good: 5.0, fair: 10.0, poor: 15.0 };

    pub const FAN_IN_MEAN: MetricThresholds =
        MetricThresholds { excellent: 0.5, good: 1.0, fair: 2.0, poor: 3.0 };

    pub const FAN_OUT_MAX: MetricThresholds =
        MetricThresholds { excellent: 3.0, good: 5.0, fair: 8.0, poor: 12.0 };

    pub const FAN_OUT_MEAN: MetricThresholds =
        MetricThresholds { excellent: 0.5, good: 1.0, fair: 2.0, poor: 3.0 };

    pub const FUNCTION_SIZE: MetricThresholds = MetricThresholds {
        excellent: 20.0,
        good: 50.0,
        fair: 100.0,
        poor: 200.0,
    };

    pub const PARAMS: MetricThresholds =
        MetricThresholds { excellent: 3.0, good: 5.0, fair: 7.0, poor: 10.0 };

    pub const CYCLOMATIC: MetricThresholds = MetricThresholds {
        excellent: 5.0,
        good: 10.0,
        fair: 20.0,
        poor: 30.0,
    };

    pub const COGNITIVE: MetricThresholds = MetricThresholds {
        excellent: 5.0,
        good: 10.0,
        fair: 15.0,
        poor: 25.0,
    };

    pub const METHODS_PER_STRUCT: MetricThresholds = MetricThresholds {
        excellent: 5.0,
        good: 10.0,
        fair: 15.0,
        poor: 25.0,
    };

    // LCOM4 = connected components. 1 = cohesive, 2+ = should split
    pub const LCOM: MetricThresholds =
        MetricThresholds { excellent: 2.0, good: 3.0, fair: 4.0, poor: 5.0 };

    pub const FILE_LOC: MetricThresholds = MetricThresholds {
        excellent: 200.0,
        good: 400.0,
        fair: 600.0,
        poor: 1000.0,
    };

    pub const MAX_SCOPE: MetricThresholds = MetricThresholds {
        excellent: 15.0,
        good: 30.0,
        fair: 50.0,
        poor: 100.0,
    };

    pub const DUPLICATION_PCT: MetricThresholds =
        MetricThresholds { excellent: 3.0, good: 5.0, fair: 10.0, poor: 20.0 };

    // Lower-is-worse: fields are the LOW boundary of each bucket.
    // value >= excellent (90) → excellent; value < poor (60) → critical.
    pub const LINE_COV: MetricThresholds = MetricThresholds {
        excellent: 90.0,
        good: 80.0,
        fair: 70.0,
        poor: 60.0,
    };

    pub const UNCOV_BRANCHES: MetricThresholds =
        MetricThresholds { excellent: 1.0, good: 3.0, fair: 6.0, poor: 10.0 };
}

impl Default for ThresholdsConfig {
    fn default() -> Self {
        Self {
            dag_density: defaults::DAG_DENSITY,
            fan_in_max: defaults::FAN_IN_MAX,
            fan_in_mean: defaults::FAN_IN_MEAN,
            fan_out_max: defaults::FAN_OUT_MAX,
            fan_out_mean: defaults::FAN_OUT_MEAN,
            function_size: defaults::FUNCTION_SIZE,
            params: defaults::PARAMS,
            cyclomatic: defaults::CYCLOMATIC,
            cognitive: defaults::COGNITIVE,
            methods_per_struct: defaults::METHODS_PER_STRUCT,
            lcom: defaults::LCOM,
            file_loc: defaults::FILE_LOC,
            max_scope: defaults::MAX_SCOPE,
            duplication_pct: defaults::DUPLICATION_PCT,
            line_cov: defaults::LINE_COV,
            uncov_branches: defaults::UNCOV_BRANCHES,
        }
    }
}

impl ThresholdsConfig {
    /// Get thresholds for a metric by name
    pub fn get(&self, name: &str) -> Option<&MetricThresholds> {
        match name {
            "dag_density" => Some(&self.dag_density),
            "fan_in" => Some(&self.fan_in_max),
            "fan_out" => Some(&self.fan_out_max),
            "function_size" => Some(&self.function_size),
            "params" => Some(&self.params),
            "cyclomatic" => Some(&self.cyclomatic),
            "cognitive" => Some(&self.cognitive),
            "methods_per_struct" => Some(&self.methods_per_struct),
            "lcom" => Some(&self.lcom),
            "file_loc" => Some(&self.file_loc),
            "max_scope" => Some(&self.max_scope),
            "duplication_pct" => Some(&self.duplication_pct),
            "line_cov" => Some(&self.line_cov),
            "uncov_branches" => Some(&self.uncov_branches),
            _ => None,
        }
    }
}

/// Hub detection thresholds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubThresholds {
    /// Minimum fan_in to be considered a hub candidate (default: 10)
    pub min_fan_in: usize,
    /// Minimum fan_out to be considered a hub (default: 3)
    pub min_fan_out: usize,
}

impl Default for HubThresholds {
    fn default() -> Self {
        Self {
            min_fan_in: mdlr_metrics::DEFAULT_HUB_MIN_FAN_IN,
            min_fan_out: mdlr_metrics::DEFAULT_HUB_MIN_FAN_OUT,
        }
    }
}

/// CPD (Copy-Paste Detection) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpdConfig {
    /// Minimum number of tokens for a block to be considered a duplicate (default: 50)
    pub min_tokens: usize,
}

impl Default for CpdConfig {
    fn default() -> Self {
        Self { min_tokens: 50 }
    }
}

/// Canonical metric names — the identifiers shown by `mdlr metrics ls`,
/// printed in the `metric` column / JSON keys, and accepted in
/// `disabled_metrics`. The single source of truth for which names are valid.
pub const METRIC_NAMES: &[&str] = &[
    "dag_density",
    "fan_in",
    "fan_out",
    "function_size",
    "params",
    "cyclomatic",
    "cognitive",
    "max_scope",
    "methods_per_struct",
    "lcom",
    "file_loc",
    "duplication_pct",
    "line_cov",
    "uncov_branches",
];

/// Main configuration struct
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub thresholds: ThresholdsConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub hub: HubThresholds,
    #[serde(default)]
    pub cpd: CpdConfig,
    /// Canonical metric names to suppress from `check` output. Disabling is an
    /// output-control concern: bundled compute passes still run, but the CPD
    /// and coverage passes are skipped when their metrics are fully disabled.
    #[serde(default)]
    pub disabled_metrics: Vec<String>,
}

impl Config {
    /// Whether a metric (by canonical name) is disabled in this config.
    pub fn is_disabled(&self, metric: &str) -> bool {
        self.disabled_metrics.iter().any(|m| m == metric)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_threshold_evaluation() {
        let thresholds = MetricThresholds {
            excellent: 0.5,
            good: 1.0,
            fair: 1.5,
            poor: 2.0,
        };

        assert_eq!(thresholds.evaluate(0.3), Bucket::Excellent);
        assert_eq!(thresholds.evaluate(0.5), Bucket::Good);
        assert_eq!(thresholds.evaluate(0.7), Bucket::Good);
        assert_eq!(thresholds.evaluate(1.0), Bucket::Fair);
        assert_eq!(thresholds.evaluate(1.3), Bucket::Fair);
        assert_eq!(thresholds.evaluate(1.5), Bucket::Poor);
        assert_eq!(thresholds.evaluate(1.8), Bucket::Poor);
        assert_eq!(thresholds.evaluate(2.0), Bucket::Critical);
        assert_eq!(thresholds.evaluate(5.0), Bucket::Critical);
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.display.mode, DisplayMode::Both);
        assert_eq!(config.thresholds.dag_density.excellent, 0.5);
        assert!(config.disabled_metrics.is_empty());
    }

    #[test]
    fn test_is_disabled() {
        let config = Config {
            disabled_metrics: vec!["lcom".to_string(), "fan_in".to_string()],
            ..Default::default()
        };
        assert!(config.is_disabled("lcom"));
        assert!(config.is_disabled("fan_in"));
        assert!(!config.is_disabled("cyclomatic"));
    }

    #[test]
    fn metric_names_cover_all_thresholds() {
        // Every threshold-backed metric name must be a recognized canonical
        // name so it can be disabled.
        for name in
            ["dag_density", "fan_in", "lcom", "duplication_pct", "line_cov"]
        {
            assert!(METRIC_NAMES.contains(&name), "missing {name}");
        }
    }
}
