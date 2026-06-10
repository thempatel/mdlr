//! Quality buckets and the threshold tables that map metric values onto
//! them.

use serde::{Deserialize, Serialize};

/// Bucket labels for metric severity
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

/// Thresholds for a single metric
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Thresholds for the structural metrics evaluated by
/// [`crate::display::BucketedMetrics::from_metrics`].
#[derive(Debug, Clone)]
pub struct Thresholds {
    pub dag_density: MetricThresholds,
    pub fan_in_max: MetricThresholds,
    pub fan_in_mean: MetricThresholds,
    pub fan_out_max: MetricThresholds,
    pub fan_out_mean: MetricThresholds,
}
