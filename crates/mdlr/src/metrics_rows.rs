//! Metric row collection for CLI output.

use crate::cache::Ignores;
use crate::config::{Bucket, Config, MetricThresholds};
use mdlr_metrics::{
    ComplexityMetrics, FileLocMetrics, HubInfo, StructMetrics,
    StructuralMetrics, TagMetrics,
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
    pub tag_metrics: &'a TagMetrics,
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

/// Collect metric rows for text output.
///
/// The `k` parameter limits how many rows are collected globally:
/// - If `k < 0`, all rows are collected
/// - If `k >= 0`, at most `k` rows are collected total, prioritizing by severity
///
/// Rows are selected by severity (critical first, then poor, fair, good, excellent)
/// across all metric types, then grouped by metric type for display.
///
/// The `symbol_filter` parameter, when `Some`, limits output to only the
/// matching symbol. This is used when filtering by a specific symbol ID.
///
/// The `ignores` parameter filters out metrics that have been explicitly ignored.
pub fn collect_metric_rows(
    metrics: &MetricsBundle,
    config: &Config,
    k: i32,
    symbol_filter: Option<&str>,
    ignores: &Ignores,
) -> Vec<MetricRow> {
    let t = &config.thresholds;
    let m = metrics;

    // Integer metrics (excluding fan_in which has special hub filtering)
    let int_specs = [
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
            name: "methods_per_struct",
            distribution: &m.struct_metrics.methods_per_struct.distribution,
            thresholds: &t.methods_per_struct,
            min_value: 0,
        },
        IntMetricSpec {
            name: "file_loc",
            distribution: &m.file_loc.distribution,
            thresholds: &t.file_loc,
            min_value: 0,
        },
    ];

    // Hub-filtered fan_in metric (only flags units with high fan_in AND high fan_out)
    let fan_in_spec = HubFilteredFanInSpec {
        distribution: &m.structural.fan_in.distribution,
        thresholds: &t.fan_in_max,
        hubs: &m.structural.hubs,
    };

    // Float metrics
    let float_specs = [FloatMetricSpec {
        name: "lcom",
        distribution: &m.struct_metrics.lcom.distribution,
        thresholds: &t.lcom,
        min_value: 0.0,
    }];

    // Handle symbol filter mode separately (no global sorting)
    if let Some(filter) = symbol_filter {
        let mut rows: Vec<MetricRow> = Vec::new();
        for spec in &int_specs {
            spec.collect_filtered(&mut rows, filter);
        }
        fan_in_spec.collect_filtered(&mut rows, filter);
        for spec in &float_specs {
            spec.collect_filtered(&mut rows, filter);
        }
        // Filter out ignored metrics
        rows.retain(|(metric, symbol, _, _)| {
            !ignores.is_ignored(symbol, metric)
        });
        return rows;
    }

    // Collect all rows with severity scores
    let mut scored_rows: Vec<ScoredRow> = Vec::new();
    for spec in &int_specs {
        spec.collect_all(&mut scored_rows);
    }
    fan_in_spec.collect_all(&mut scored_rows);
    for spec in &float_specs {
        spec.collect_all(&mut scored_rows);
    }

    // Filter out ignored metrics before sorting
    scored_rows
        .retain(|row| !ignores.is_ignored(&row.symbol, &row.metric_name));

    // Sort by severity descending (worst first)
    scored_rows.sort_by(|a, b| b.severity().cmp(&a.severity()));

    // Apply global limit
    let selected: Vec<ScoredRow> = if k < 0 {
        scored_rows
    } else {
        scored_rows.into_iter().take(k as usize).collect()
    };

    // Group by metric type to maintain display grouping
    let mut grouped: std::collections::HashMap<String, Vec<ScoredRow>> =
        std::collections::HashMap::new();
    for row in selected {
        grouped.entry(row.metric_name.clone()).or_default().push(row);
    }

    // Define metric order for consistent output
    let metric_order = [
        "fan_out",
        "fan_in",
        "function_size",
        "params",
        "cyclomatic",
        "methods_per_struct",
        "file_loc",
        "lcom",
    ];

    // Convert to MetricRows in metric order
    let mut rows: Vec<MetricRow> = Vec::new();
    for metric_name in &metric_order {
        if let Some(metric_rows) = grouped.remove(*metric_name) {
            for row in metric_rows {
                rows.push(row.into_row());
            }
        }
    }

    // Conceptual metrics (if tags exist)
    collect_conceptual_metrics(&mut rows, m, k);

    rows
}

/// Collect conceptual metrics from tag metrics.
fn collect_conceptual_metrics(
    rows: &mut Vec<MetricRow>,
    m: &MetricsBundle,
    k: i32,
) {
    let Some(ref conceptual) = m.tag_metrics.conceptual else {
        return;
    };

    let limit_fan_out = if k < 0 {
        conceptual.conceptual_fan_out.top.len()
    } else {
        k as usize
    };
    for (name, count) in
        conceptual.conceptual_fan_out.top.iter().take(limit_fan_out)
    {
        if *count > 1 {
            rows.push((
                "conceptual_fan_out".to_string(),
                name.clone(),
                count.to_string(),
                "-".to_string(),
            ));
        }
    }

    let limit_scatter =
        if k < 0 { conceptual.concept_scattering.len() } else { k as usize };
    for scatter in conceptual.concept_scattering.iter().take(limit_scatter) {
        if scatter.file_count > 1 {
            rows.push((
                "concept_scattering".to_string(),
                scatter.tag.clone(),
                format!("{:.2}", scatter.scatter_ratio),
                "-".to_string(),
            ));
        }
    }
}
