//! Metric row collection for CLI output.

use crate::cache::Ignores;
use crate::config::{Bucket, Config, MetricThresholds};
use mdlr_metrics::{
    ComplexityMetrics, FileLocMetrics, HubInfo, StructMetrics,
    StructuralMetrics,
};
use std::collections::HashMap;

/// A metric row: (metric_name, symbol, value, bucket)
pub type MetricRow = (String, String, String, String);

/// Internal representation with bucket for sorting
struct ScoredRow {
    metric_name: String,
    symbol: String,
    value: String,
    bucket: Bucket,
}

impl ScoredRow {
    /// Convert to MetricRow for output
    fn into_row(self) -> MetricRow {
        (self.metric_name, self.symbol, self.value, self.bucket.to_string())
    }

    /// Severity score for sorting (higher = worse)
    fn severity(&self) -> u8 {
        match self.bucket {
            Bucket::Excellent => 0,
            Bucket::Good => 1,
            Bucket::Fair => 2,
            Bucket::Poor => 3,
            Bucket::Critical => 4,
        }
    }
}

/// Bundle of all computed metrics for collection
pub struct MetricsBundle<'a> {
    pub structural: &'a StructuralMetrics,
    pub complexity: &'a ComplexityMetrics,
    pub struct_metrics: &'a StructMetrics,
    pub file_loc: &'a FileLocMetrics,
}

/// Specification for collecting an integer metric
struct IntMetricSpec<'a> {
    name: &'static str,
    distribution: &'a [(String, usize)],
    thresholds: &'a MetricThresholds,
    min_value: usize,
}

impl IntMetricSpec<'_> {
    /// Collect all entries (for global sorting mode)
    fn collect_all(&self, rows: &mut Vec<ScoredRow>) {
        for (name, value) in self.distribution {
            if *value > self.min_value {
                let bucket = self.thresholds.evaluate(*value as f64);
                rows.push(ScoredRow {
                    metric_name: self.name.to_string(),
                    symbol: name.clone(),
                    value: value.to_string(),
                    bucket,
                });
            }
        }
    }

    /// Collect with per-metric limit (for symbol filter mode)
    fn collect_filtered(
        &self,
        rows: &mut Vec<MetricRow>,
        symbol_filter: &str,
    ) {
        for (name, value) in self.distribution.iter() {
            if name == symbol_filter && *value > self.min_value {
                let bucket = self.thresholds.evaluate(*value as f64);
                rows.push((
                    self.name.to_string(),
                    name.clone(),
                    value.to_string(),
                    bucket.to_string(),
                ));
            }
        }
    }
}

/// Specification for collecting fan_in metric with hub filtering
/// Only includes units that are hubs (high fan_in AND high fan_out)
struct HubFilteredFanInSpec<'a> {
    distribution: &'a [(String, usize)],
    thresholds: &'a MetricThresholds,
    hubs: &'a HashMap<String, HubInfo>,
}

impl HubFilteredFanInSpec<'_> {
    /// Collect only hub entries (for global sorting mode)
    fn collect_all(&self, rows: &mut Vec<ScoredRow>) {
        for (name, value) in self.distribution {
            // Only include if this unit is a hub
            if self.hubs.contains_key(name) {
                let bucket = self.thresholds.evaluate(*value as f64);
                rows.push(ScoredRow {
                    metric_name: "fan_in".to_string(),
                    symbol: name.clone(),
                    value: value.to_string(),
                    bucket,
                });
            }
        }
    }

    /// Collect with per-metric limit (for symbol filter mode)
    /// In symbol filter mode, always show the value regardless of hub status
    fn collect_filtered(
        &self,
        rows: &mut Vec<MetricRow>,
        symbol_filter: &str,
    ) {
        for (name, value) in self.distribution.iter() {
            if name == symbol_filter {
                let bucket = self.thresholds.evaluate(*value as f64);
                rows.push((
                    "fan_in".to_string(),
                    name.clone(),
                    value.to_string(),
                    bucket.to_string(),
                ));
            }
        }
    }
}

/// Specification for collecting a float metric
struct FloatMetricSpec<'a> {
    name: &'static str,
    distribution: &'a [(String, f64)],
    thresholds: &'a MetricThresholds,
    min_value: f64,
}

impl FloatMetricSpec<'_> {
    /// Collect all entries (for global sorting mode)
    fn collect_all(&self, rows: &mut Vec<ScoredRow>) {
        for (name, value) in self.distribution {
            if *value > self.min_value {
                let bucket = self.thresholds.evaluate(*value);
                rows.push(ScoredRow {
                    metric_name: self.name.to_string(),
                    symbol: name.clone(),
                    value: format!("{:.2}", value),
                    bucket,
                });
            }
        }
    }

    /// Collect with per-metric limit (for symbol filter mode)
    fn collect_filtered(
        &self,
        rows: &mut Vec<MetricRow>,
        symbol_filter: &str,
    ) {
        for (name, value) in self.distribution.iter() {
            if name == symbol_filter && *value > self.min_value {
                let bucket = self.thresholds.evaluate(*value);
                rows.push((
                    self.name.to_string(),
                    name.clone(),
                    format!("{:.2}", value),
                    bucket.to_string(),
                ));
            }
        }
    }
}

/// Bundled metric specifications for collection
struct MetricSpecs<'a> {
    int_specs: Vec<IntMetricSpec<'a>>,
    fan_in_spec: HubFilteredFanInSpec<'a>,
    lcom_spec: IntMetricSpec<'a>,
    float_specs: Vec<FloatMetricSpec<'a>>,
}

impl<'a> MetricSpecs<'a> {
    fn new(m: &'a MetricsBundle, config: &'a Config) -> Self {
        let t = &config.thresholds;
        MetricSpecs {
            int_specs: vec![
                IntMetricSpec {
                    name: "fan_out",
                    distribution: &m.structural.fan_out.distribution,
                    thresholds: &t.fan_out_max,
                    min_value: 0,
                },
                IntMetricSpec {
                    name: "function_size",
                    distribution: &m.complexity.size.distribution,
                    thresholds: &t.function_size,
                    min_value: 1,
                },
                IntMetricSpec {
                    name: "params",
                    distribution: &m.complexity.params.distribution,
                    thresholds: &t.params,
                    min_value: 0,
                },
                IntMetricSpec {
                    name: "cyclomatic",
                    distribution: &m.complexity.cyclomatic.distribution,
                    thresholds: &t.cyclomatic,
                    min_value: 1,
                },
                IntMetricSpec {
                    name: "max_scope",
                    distribution: &m.complexity.max_scope.distribution,
                    thresholds: &t.max_scope,
                    min_value: 0,
                },
                IntMetricSpec {
                    name: "methods_per_struct",
                    distribution: &m
                        .struct_metrics
                        .methods_per_struct
                        .distribution,
                    thresholds: &t.methods_per_struct,
                    min_value: 0,
                },
                IntMetricSpec {
                    name: "file_loc",
                    distribution: &m.file_loc.distribution,
                    thresholds: &t.file_loc,
                    min_value: 0,
                },
            ],
            fan_in_spec: HubFilteredFanInSpec {
                distribution: &m.structural.fan_in.distribution,
                thresholds: &t.fan_in_max,
                hubs: &m.structural.hubs,
            },
            lcom_spec: IntMetricSpec {
                name: "lcom",
                distribution: &m.struct_metrics.lcom.distribution,
                thresholds: &t.lcom,
                min_value: 0,
            },
            float_specs: vec![],
        }
    }

    fn collect_filtered(&self, filter: &str) -> Vec<MetricRow> {
        let mut rows = Vec::new();
        for spec in &self.int_specs {
            spec.collect_filtered(&mut rows, filter);
        }
        self.fan_in_spec.collect_filtered(&mut rows, filter);
        self.lcom_spec.collect_filtered(&mut rows, filter);
        for spec in &self.float_specs {
            spec.collect_filtered(&mut rows, filter);
        }
        rows
    }

    fn collect_all_scored(&self) -> Vec<ScoredRow> {
        let mut rows = Vec::new();
        for spec in &self.int_specs {
            spec.collect_all(&mut rows);
        }
        self.fan_in_spec.collect_all(&mut rows);
        self.lcom_spec.collect_all(&mut rows);
        for spec in &self.float_specs {
            spec.collect_all(&mut rows);
        }
        rows
    }
}

/// Canonical metric display order
const METRIC_ORDER: &[&str] = &[
    "fan_out",
    "fan_in",
    "function_size",
    "params",
    "cyclomatic",
    "max_scope",
    "methods_per_struct",
    "file_loc",
    "lcom",
];

/// Sort scored rows by severity, apply limit, then group by metric in canonical order.
fn sort_and_group(mut scored_rows: Vec<ScoredRow>, k: i32) -> Vec<MetricRow> {
    scored_rows.sort_by(|a, b| b.severity().cmp(&a.severity()));

    let selected: Vec<ScoredRow> = if k < 0 {
        scored_rows
    } else {
        scored_rows.into_iter().take(k as usize).collect()
    };

    let mut grouped: HashMap<String, Vec<ScoredRow>> = HashMap::new();
    for row in selected {
        grouped.entry(row.metric_name.clone()).or_default().push(row);
    }

    let mut rows = Vec::new();
    for metric_name in METRIC_ORDER {
        if let Some(metric_rows) = grouped.remove(*metric_name) {
            for row in metric_rows {
                rows.push(row.into_row());
            }
        }
    }
    rows
}

/// Collect metric rows for text output.
///
/// The `k` parameter limits how many rows are collected globally:
/// - If `k < 0`, all rows are collected
/// - If `k >= 0`, at most `k` rows are collected total, prioritizing by severity
///
/// Rows are selected by severity (critical first, then poor, fair, good, excellent)
/// across all metric types, then grouped by metric type for display.
pub fn collect_metric_rows(
    metrics: &MetricsBundle,
    config: &Config,
    k: i32,
    symbol_filter: Option<&str>,
    ignores: &Ignores,
) -> Vec<MetricRow> {
    let specs = MetricSpecs::new(metrics, config);

    if let Some(filter) = symbol_filter {
        let mut rows = specs.collect_filtered(filter);
        rows.retain(|(metric, symbol, _, _)| {
            !ignores.is_ignored(symbol, metric)
        });
        return rows;
    }

    let mut scored_rows = specs.collect_all_scored();
    scored_rows
        .retain(|row| !ignores.is_ignored(&row.symbol, &row.metric_name));

    sort_and_group(scored_rows, k)
}
