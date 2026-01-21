use crate::config::{Bucket, Config, DisplayMode};
use super::StructuralMetrics;
use serde::Serialize;

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

impl BucketedMetrics {
    /// Evaluate metrics against thresholds from config
    pub fn from_metrics(metrics: &StructuralMetrics, config: &Config) -> Self {
        let thresholds = &config.thresholds;

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
                    thresholds.fan_out_max.evaluate(metrics.fan_out.max as f64),
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
fn format_value(value: &BucketedValue, config: &Config, is_integer: bool) -> String {
    let label = config.labels.get(value.bucket);

    match config.display.mode {
        DisplayMode::Value => {
            if is_integer {
                format!("{}", value.value as usize)
            } else {
                format!("{:.3}", value.value)
            }
        }
        DisplayMode::Label => label.to_string(),
        DisplayMode::Both => {
            if is_integer {
                format!("{} ({})", value.value as usize, label)
            } else {
                format!("{:.3} ({})", value.value, label)
            }
        }
    }
}

/// Formats a fan metric (max + mean) according to display mode
fn format_fan_metric(fan: &BucketedFanMetrics, config: &Config) -> String {
    let max_str = format_value(&fan.max, config, true);
    let mean_str = format_value(&fan.mean, config, false);

    match config.display.mode {
        DisplayMode::Value => format!("max={}, mean={:.2}", fan.max.value as usize, fan.mean.value),
        DisplayMode::Label => format!("max={}, mean={}", max_str, mean_str),
        DisplayMode::Both => format!("max={}, mean={}", max_str, mean_str),
    }
}

/// Display wrapper that formats metrics with bucket labels
pub struct MetricsDisplay<'a> {
    pub metrics: &'a StructuralMetrics,
    pub bucketed: BucketedMetrics,
    pub config: &'a Config,
}

impl<'a> MetricsDisplay<'a> {
    pub fn new(metrics: &'a StructuralMetrics, config: &'a Config) -> Self {
        let bucketed = BucketedMetrics::from_metrics(metrics, config);
        Self {
            metrics,
            bucketed,
            config,
        }
    }
}

impl std::fmt::Display for MetricsDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Structural Metrics")?;
        writeln!(f, "==================")?;
        writeln!(f)?;

        let dag_str = format_value(&self.bucketed.dag_density, self.config, false);
        writeln!(f, "DAG Density: {}", dag_str)?;
        writeln!(f)?;

        let fan_in_str = format_fan_metric(&self.bucketed.fan_in, self.config);
        let fan_out_str = format_fan_metric(&self.bucketed.fan_out, self.config);
        writeln!(f, "Fan-In:  {}", fan_in_str)?;
        writeln!(f, "Fan-Out: {}", fan_out_str)?;

        if !self.metrics.fan_out.distribution.is_empty() {
            writeln!(f)?;
            writeln!(f, "Top Fan-Out:")?;
            for (name, count) in self.metrics.fan_out.distribution.iter().take(10) {
                if *count > 0 {
                    writeln!(f, "  {} ({})", name, count)?;
                }
            }
        }

        if !self.metrics.fan_in.distribution.is_empty() {
            writeln!(f)?;
            writeln!(f, "Top Fan-In:")?;
            for (name, count) in self.metrics.fan_in.distribution.iter().take(10) {
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
    use crate::config::Config;
    use crate::metrics::FanMetrics;

    fn make_test_metrics() -> StructuralMetrics {
        StructuralMetrics {
            dag_density: 0.419,
            fan_in: FanMetrics {
                max: 4,
                mean: 0.43,
                distribution: vec![],
            },
            fan_out: FanMetrics {
                max: 6,
                mean: 0.43,
                distribution: vec![],
            },
        }
    }

    #[test]
    fn test_bucketed_metrics() {
        let metrics = make_test_metrics();
        let config = Config::default();
        let bucketed = BucketedMetrics::from_metrics(&metrics, &config);

        assert_eq!(bucketed.dag_density.bucket, Bucket::Excellent);
        assert_eq!(bucketed.fan_in.max.bucket, Bucket::Good);
        assert_eq!(bucketed.fan_in.mean.bucket, Bucket::Excellent);
        assert_eq!(bucketed.fan_out.max.bucket, Bucket::Fair);
        assert_eq!(bucketed.fan_out.mean.bucket, Bucket::Excellent);
    }

    #[test]
    fn test_display_both_mode() {
        let metrics = make_test_metrics();
        let config = Config::default();
        let display = MetricsDisplay::new(&metrics, &config);
        let output = format!("{}", display);

        assert!(output.contains("DAG Density: 0.419 (excellent)"));
        assert!(output.contains("Fan-In:  max=4 (good), mean=0.430 (excellent)"));
        assert!(output.contains("Fan-Out: max=6 (fair), mean=0.430 (excellent)"));
    }

    #[test]
    fn test_display_value_mode() {
        let metrics = make_test_metrics();
        let mut config = Config::default();
        config.display.mode = DisplayMode::Value;
        let display = MetricsDisplay::new(&metrics, &config);
        let output = format!("{}", display);

        assert!(output.contains("DAG Density: 0.419"));
        assert!(!output.contains("(excellent)"));
    }

    #[test]
    fn test_display_label_mode() {
        let metrics = make_test_metrics();
        let mut config = Config::default();
        config.display.mode = DisplayMode::Label;
        let display = MetricsDisplay::new(&metrics, &config);
        let output = format!("{}", display);

        assert!(output.contains("DAG Density: excellent"));
        assert!(output.contains("Fan-In:  max=good, mean=excellent"));
    }

    #[test]
    fn test_custom_labels() {
        let metrics = make_test_metrics();
        let mut config = Config::default();
        config.labels.excellent = "superb".to_string();
        let display = MetricsDisplay::new(&metrics, &config);
        let output = format!("{}", display);

        assert!(output.contains("(superb)"));
    }
}
