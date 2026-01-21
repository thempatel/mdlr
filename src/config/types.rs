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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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

/// Custom labels for buckets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketLabels {
    #[serde(default = "default_excellent")]
    pub excellent: String,
    #[serde(default = "default_good")]
    pub good: String,
    #[serde(default = "default_fair")]
    pub fair: String,
    #[serde(default = "default_poor")]
    pub poor: String,
    #[serde(default = "default_critical")]
    pub critical: String,
}

fn default_excellent() -> String {
    "excellent".to_string()
}
fn default_good() -> String {
    "good".to_string()
}
fn default_fair() -> String {
    "fair".to_string()
}
fn default_poor() -> String {
    "poor".to_string()
}
fn default_critical() -> String {
    "critical".to_string()
}

impl Default for BucketLabels {
    fn default() -> Self {
        Self {
            excellent: default_excellent(),
            good: default_good(),
            fair: default_fair(),
            poor: default_poor(),
            critical: default_critical(),
        }
    }
}

impl BucketLabels {
    pub fn get(&self, bucket: Bucket) -> &str {
        match bucket {
            Bucket::Excellent => &self.excellent,
            Bucket::Good => &self.good,
            Bucket::Fair => &self.fair,
            Bucket::Poor => &self.poor,
            Bucket::Critical => &self.critical,
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
        Self {
            mode: DisplayMode::Both,
        }
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
}

fn default_dag_density() -> MetricThresholds {
    MetricThresholds {
        excellent: 0.5,
        good: 1.0,
        fair: 1.5,
        poor: 2.0,
    }
}

fn default_fan_in_max() -> MetricThresholds {
    MetricThresholds {
        excellent: 3.0,
        good: 5.0,
        fair: 10.0,
        poor: 15.0,
    }
}

fn default_fan_in_mean() -> MetricThresholds {
    MetricThresholds {
        excellent: 0.5,
        good: 1.0,
        fair: 2.0,
        poor: 3.0,
    }
}

fn default_fan_out_max() -> MetricThresholds {
    MetricThresholds {
        excellent: 3.0,
        good: 5.0,
        fair: 8.0,
        poor: 12.0,
    }
}

fn default_fan_out_mean() -> MetricThresholds {
    MetricThresholds {
        excellent: 0.5,
        good: 1.0,
        fair: 2.0,
        poor: 3.0,
    }
}

impl Default for ThresholdsConfig {
    fn default() -> Self {
        Self {
            dag_density: default_dag_density(),
            fan_in_max: default_fan_in_max(),
            fan_in_mean: default_fan_in_mean(),
            fan_out_max: default_fan_out_max(),
            fan_out_mean: default_fan_out_mean(),
        }
    }
}

/// Main configuration struct
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub labels: BucketLabels,
    #[serde(default)]
    pub thresholds: ThresholdsConfig,
    #[serde(default)]
    pub display: DisplayConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            labels: BucketLabels::default(),
            thresholds: ThresholdsConfig::default(),
            display: DisplayConfig::default(),
        }
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
        assert_eq!(config.labels.excellent, "excellent");
        assert_eq!(config.display.mode, DisplayMode::Both);
        assert_eq!(config.thresholds.dag_density.excellent, 0.5);
    }

    #[test]
    fn test_bucket_labels() {
        let labels = BucketLabels::default();
        assert_eq!(labels.get(Bucket::Excellent), "excellent");
        assert_eq!(labels.get(Bucket::Critical), "critical");
    }
}
