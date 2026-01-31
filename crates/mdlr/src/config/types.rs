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

impl Bucket {
    /// Returns all buckets in order from best to worst
    pub fn all() -> &'static [Bucket] {
        &[
            Bucket::Excellent,
            Bucket::Good,
            Bucket::Fair,
            Bucket::Poor,
            Bucket::Critical,
        ]
    }
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
    /// Evaluate a value against thresholds to get a bucket
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

/// All thresholds configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdsConfig {
    #[serde(default = "default_dag_density")]
    pub dag_density: MetricThresholds,
    #[serde(default = "default_fan_in_max")]
    pub fan_in_max: MetricThresholds,
    #[serde(default = "default_fan_in_mean")]
    pub fan_in_mean: MetricThresholds,
    #[serde(default = "default_fan_out_max")]
    pub fan_out_max: MetricThresholds,
    #[serde(default = "default_fan_out_mean")]
    pub fan_out_mean: MetricThresholds,
    #[serde(default = "default_function_size")]
    pub function_size: MetricThresholds,
    #[serde(default = "default_params")]
    pub params: MetricThresholds,
    #[serde(default = "default_cyclomatic")]
    pub cyclomatic: MetricThresholds,
    #[serde(default = "default_methods_per_struct")]
    pub methods_per_struct: MetricThresholds,
    #[serde(default = "default_lcom")]
    pub lcom: MetricThresholds,
    #[serde(default = "default_file_loc")]
    pub file_loc: MetricThresholds,
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

    pub const METHODS_PER_STRUCT: MetricThresholds = MetricThresholds {
        excellent: 5.0,
        good: 10.0,
        fair: 15.0,
        poor: 25.0,
    };

    // LCOM is normalized 0-1, higher = less cohesive
    pub const LCOM: MetricThresholds =
        MetricThresholds { excellent: 0.2, good: 0.4, fair: 0.6, poor: 0.8 };

    pub const FILE_LOC: MetricThresholds = MetricThresholds {
        excellent: 200.0,
        good: 400.0,
        fair: 600.0,
        poor: 1000.0,
    };
}

// Serde default functions (required for partial deserialization)
fn default_dag_density() -> MetricThresholds {
    defaults::DAG_DENSITY
}
fn default_fan_in_max() -> MetricThresholds {
    defaults::FAN_IN_MAX
}
fn default_fan_in_mean() -> MetricThresholds {
    defaults::FAN_IN_MEAN
}
fn default_fan_out_max() -> MetricThresholds {
    defaults::FAN_OUT_MAX
}
fn default_fan_out_mean() -> MetricThresholds {
    defaults::FAN_OUT_MEAN
}
fn default_function_size() -> MetricThresholds {
    defaults::FUNCTION_SIZE
}
fn default_params() -> MetricThresholds {
    defaults::PARAMS
}
fn default_cyclomatic() -> MetricThresholds {
    defaults::CYCLOMATIC
}
fn default_methods_per_struct() -> MetricThresholds {
    defaults::METHODS_PER_STRUCT
}
fn default_lcom() -> MetricThresholds {
    defaults::LCOM
}
fn default_file_loc() -> MetricThresholds {
    defaults::FILE_LOC
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
            methods_per_struct: defaults::METHODS_PER_STRUCT,
            lcom: defaults::LCOM,
            file_loc: defaults::FILE_LOC,
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
            "methods_per_struct" => Some(&self.methods_per_struct),
            "lcom" => Some(&self.lcom),
            "file_loc" => Some(&self.file_loc),
            _ => None,
        }
    }
}

/// Main configuration struct
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub thresholds: ThresholdsConfig,
    #[serde(default)]
    pub display: DisplayConfig,
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
    }
}
