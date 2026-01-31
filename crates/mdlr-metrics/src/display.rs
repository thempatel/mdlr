use super::StructuralMetrics;
use serde::Serialize;

/// Bucket labels for metric severity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

/// A metric value with its evaluated bucket
#[derive(Debug, Clone, Serialize)]
pub struct BucketedValue {
    pub value: f64,
    pub bucket: Bucket,
}

impl BucketedValue {
    pub fn new(value: f64, bucket: Bucket) -> Self {
        Self { value, bucket }
    }
}

/// Bucketed fan metrics
#[derive(Debug, Clone, Serialize)]
pub struct BucketedFanMetrics {
    pub max: BucketedValue,
    pub mean: BucketedValue,
}

/// All metrics with bucket labels attached
#[derive(Debug, Clone, Serialize)]
pub struct BucketedMetrics {
    pub dag_density: BucketedValue,
    pub fan_in: BucketedFanMetrics,
    pub fan_out: BucketedFanMetrics,
}

/// Thresholds for a single metric
#[derive(Debug, Clone)]
pub struct MetricThresholds {
    pub excellent: f64,
    pub good: f64,
    pub fair: f64,
    pub poor: f64,
}

impl MetricThresholds {
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

impl Default for MetricThresholds {
    fn default() -> Self {
        Self { excellent: 1.0, good: 2.0, fair: 3.0, poor: 5.0 }
    }
}

/// Display mode for metric output
#[derive(Debug, Clone, Copy, Default)]
pub enum DisplayMode {
    Value,
    Label,
    #[default]
    Both,
}

/// Thresholds for all metrics
#[derive(Debug, Clone)]
pub struct Thresholds {
    pub dag_density: MetricThresholds,
    pub fan_in_max: MetricThresholds,
    pub fan_in_mean: MetricThresholds,
    pub fan_out_max: MetricThresholds,
    pub fan_out_mean: MetricThresholds,
    pub function_size: MetricThresholds,
    pub params: MetricThresholds,
    pub cyclomatic: MetricThresholds,
    pub methods_per_struct: MetricThresholds,
    pub lcom: MetricThresholds,
    pub file_loc: MetricThresholds,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            dag_density: MetricThresholds {
                excellent: 0.5,
                good: 1.0,
                fair: 2.0,
                poor: 3.0,
            },
            fan_in_max: MetricThresholds {
                excellent: 3.0,
                good: 5.0,
                fair: 10.0,
                poor: 15.0,
            },
            fan_in_mean: MetricThresholds {
                excellent: 1.0,
                good: 2.0,
                fair: 3.0,
                poor: 5.0,
            },
            fan_out_max: MetricThresholds {
                excellent: 3.0,
                good: 5.0,
                fair: 8.0,
                poor: 12.0,
            },
            fan_out_mean: MetricThresholds {
                excellent: 1.0,
                good: 2.0,
                fair: 3.0,
                poor: 5.0,
            },
            function_size: MetricThresholds {
                excellent: 20.0,
                good: 50.0,
                fair: 100.0,
                poor: 200.0,
            },
            params: MetricThresholds {
                excellent: 2.0,
                good: 4.0,
                fair: 6.0,
                poor: 8.0,
            },
            cyclomatic: MetricThresholds {
                excellent: 5.0,
                good: 10.0,
                fair: 20.0,
                poor: 40.0,
            },
            methods_per_struct: MetricThresholds {
                excellent: 5.0,
                good: 10.0,
                fair: 15.0,
                poor: 25.0,
            },
            lcom: MetricThresholds {
                excellent: 0.2,
                good: 0.4,
                fair: 0.6,
                poor: 0.8,
            },
            file_loc: MetricThresholds {
                excellent: 200.0,
                good: 500.0,
                fair: 1000.0,
                poor: 2000.0,
            },
        }
    }
}

impl BucketedMetrics {
    /// Evaluate metrics against thresholds
    pub fn from_metrics(
        metrics: &StructuralMetrics,
        thresholds: &Thresholds,
    ) -> Self {
        Self {
            dag_density: BucketedValue::new(
                metrics.dag_density,
                thresholds.dag_density.evaluate(metrics.dag_density),
            ),
            fan_in: BucketedFanMetrics {
                max: BucketedValue::new(
                    metrics.fan_in.max as f64,
                    thresholds.fan_in_max.evaluate(metrics.fan_in.max as f64),
                ),
                mean: BucketedValue::new(
                    metrics.fan_in.mean,
                    thresholds.fan_in_mean.evaluate(metrics.fan_in.mean),
                ),
            },
            fan_out: BucketedFanMetrics {
                max: BucketedValue::new(
                    metrics.fan_out.max as f64,
                    thresholds
                        .fan_out_max
                        .evaluate(metrics.fan_out.max as f64),
                ),
                mean: BucketedValue::new(
                    metrics.fan_out.mean,
                    thresholds.fan_out_mean.evaluate(metrics.fan_out.mean),
                ),
            },
        }
    }
}

/// Formats a bucketed value according to display mode
fn format_value(
    value: &BucketedValue,
    mode: DisplayMode,
    is_integer: bool,
) -> String {
    match mode {
        DisplayMode::Value => {
            if is_integer {
                format!("{}", value.value as usize)
            } else {
                format!("{:.3}", value.value)
            }
        }
        DisplayMode::Label => value.bucket.to_string(),
        DisplayMode::Both => {
            if is_integer {
                format!("{} ({})", value.value as usize, value.bucket)
            } else {
                format!("{:.3} ({})", value.value, value.bucket)
            }
        }
    }
}

/// Formats a fan metric (max + mean) according to display mode
fn format_fan_metric(fan: &BucketedFanMetrics, mode: DisplayMode) -> String {
    let max_str = format_value(&fan.max, mode, true);
    let mean_str = format_value(&fan.mean, mode, false);

    match mode {
        DisplayMode::Value => format!(
            "max={}, mean={:.2}",
            fan.max.value as usize, fan.mean.value
        ),
        DisplayMode::Label => format!("max={}, mean={}", max_str, mean_str),
        DisplayMode::Both => format!("max={}, mean={}", max_str, mean_str),
    }
}

/// Display wrapper that formats metrics with bucket labels
pub struct MetricsDisplay<'a> {
    pub metrics: &'a StructuralMetrics,
    pub bucketed: BucketedMetrics,
    pub mode: DisplayMode,
}

impl<'a> MetricsDisplay<'a> {
    pub fn new(
        metrics: &'a StructuralMetrics,
        thresholds: &Thresholds,
        mode: DisplayMode,
    ) -> Self {
        let bucketed = BucketedMetrics::from_metrics(metrics, thresholds);
        Self { metrics, bucketed, mode }
    }
}

impl std::fmt::Display for MetricsDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Structural Metrics")?;
        writeln!(f, "==================")?;
        writeln!(f)?;

        let dag_str =
            format_value(&self.bucketed.dag_density, self.mode, false);
        writeln!(f, "DAG Density: {}", dag_str)?;
        writeln!(f)?;

        let fan_in_str = format_fan_metric(&self.bucketed.fan_in, self.mode);
        let fan_out_str = format_fan_metric(&self.bucketed.fan_out, self.mode);
        writeln!(f, "Fan-In:  {}", fan_in_str)?;
        writeln!(f, "Fan-Out: {}", fan_out_str)?;

        if !self.metrics.fan_out.distribution.is_empty() {
            writeln!(f)?;
            writeln!(f, "Top Fan-Out:")?;
            for (name, count) in
                self.metrics.fan_out.distribution.iter().take(10)
            {
                if *count > 0 {
                    writeln!(f, "  {} ({})", name, count)?;
                }
            }
        }

        if !self.metrics.fan_in.distribution.is_empty() {
            writeln!(f)?;
            writeln!(f, "Top Fan-In:")?;
            for (name, count) in
                self.metrics.fan_in.distribution.iter().take(10)
            {
                if *count > 0 {
                    writeln!(f, "  {} ({})", name, count)?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FanMetrics;

    fn make_test_metrics() -> StructuralMetrics {
        StructuralMetrics {
            dag_density: 0.419,
            fan_in: FanMetrics { max: 4, mean: 0.43, distribution: vec![] },
            fan_out: FanMetrics { max: 6, mean: 0.43, distribution: vec![] },
        }
    }

    #[test]
    fn test_bucketed_metrics() {
        let metrics = make_test_metrics();
        let thresholds = Thresholds::default();
        let bucketed = BucketedMetrics::from_metrics(&metrics, &thresholds);

        assert_eq!(bucketed.dag_density.bucket, Bucket::Excellent);
        assert_eq!(bucketed.fan_in.max.bucket, Bucket::Good);
        assert_eq!(bucketed.fan_in.mean.bucket, Bucket::Excellent);
        assert_eq!(bucketed.fan_out.max.bucket, Bucket::Fair);
        assert_eq!(bucketed.fan_out.mean.bucket, Bucket::Excellent);
    }

    #[test]
    fn test_display_both_mode() {
        let metrics = make_test_metrics();
        let thresholds = Thresholds::default();
        let display =
            MetricsDisplay::new(&metrics, &thresholds, DisplayMode::Both);
        let output = format!("{}", display);

        assert!(output.contains("DAG Density: 0.419 (excellent)"));
        assert!(
            output.contains("Fan-In:  max=4 (good), mean=0.430 (excellent)")
        );
        assert!(
            output.contains("Fan-Out: max=6 (fair), mean=0.430 (excellent)")
        );
    }

    #[test]
    fn test_display_value_mode() {
        let metrics = make_test_metrics();
        let thresholds = Thresholds::default();
        let display =
            MetricsDisplay::new(&metrics, &thresholds, DisplayMode::Value);
        let output = format!("{}", display);

        assert!(output.contains("DAG Density: 0.419"));
        assert!(!output.contains("(excellent)"));
    }

    #[test]
    fn test_display_label_mode() {
        let metrics = make_test_metrics();
        let thresholds = Thresholds::default();
        let display =
            MetricsDisplay::new(&metrics, &thresholds, DisplayMode::Label);
        let output = format!("{}", display);

        assert!(output.contains("DAG Density: excellent"));
        assert!(output.contains("Fan-In:  max=good, mean=excellent"));
    }
}
