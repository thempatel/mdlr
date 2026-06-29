//! JSON output formatting for the CLI.

use std::collections::HashMap;

use mdlr_core::Unit;
use mdlr_metrics::{
    BucketedFanMetrics, BucketedValue, ComplexityMetrics, CoverageMetrics,
    FileLocMetrics, StructMetrics,
};

use crate::config::{Bucket, Config, MetricThresholds};

/// Map of unit id -> unit, used to attach source `file`/`lines` to each
/// distribution entry.
pub type UnitMap<'a> = HashMap<&'a str, &'a Unit>;

/// Build a unit id -> unit lookup from a slice of units.
pub fn unit_map(units: &[Unit]) -> UnitMap<'_> {
    units.iter().map(|u| (u.id.as_str(), u)).collect()
}

/// Build JSON for a bucketed metric value
pub fn build_bucketed_json(metric: &BucketedValue) -> serde_json::Value {
    serde_json::json!({
        "value": metric.value,
        "bucket": metric.bucket,
    })
}

/// Build JSON for fan metrics (fan_in/fan_out with max/mean and distribution)
pub fn build_fan_metrics_json(
    metrics: &BucketedFanMetrics,
    distribution: &[(String, usize)],
    thresholds: &MetricThresholds,
    units: &UnitMap,
) -> serde_json::Value {
    serde_json::json!({
        "max": {
            "value": metrics.max.value as usize,
            "bucket": metrics.max.bucket,
        },
        "mean": {
            "value": metrics.mean.value,
            "bucket": metrics.mean.bucket,
        },
        "distribution": distribution_json(
            distribution, "id", "count", units,
            |_, v| thresholds.evaluate(v as f64),
        ),
    })
}

/// Build one distribution array. Each entry carries the entry key (`id_key`),
/// its `value_key`, a severity `bucket` (from `bucket_of`), and — when the
/// entry's id resolves to a unit — the source `file` and `lines` span.
/// `file_loc` entries are keyed by file path (not a unit id) so they omit
/// `file`/`lines` from the unit lookup.
pub(crate) fn distribution_json(
    distribution: &[(String, usize)],
    id_key: &str,
    value_key: &str,
    units: &UnitMap,
    bucket_of: impl Fn(&str, usize) -> Bucket,
) -> Vec<serde_json::Value> {
    distribution
        .iter()
        .map(|(id, val)| {
            let mut entry = serde_json::json!({
                id_key: id,
                value_key: val,
                "bucket": bucket_of(id, *val),
            });
            if let Some(unit) = units.get(id.as_str()) {
                entry["file"] = serde_json::json!(unit.file.to_string_lossy());
                entry["lines"] = serde_json::json!({
                    "start": unit.span.start_line,
                    "end": unit.span.end_line,
                });
            }
            entry
        })
        .collect()
}

/// Build JSON for complexity metrics with distributions
pub fn build_complexity_json(
    complexity: &ComplexityMetrics,
    config: &Config,
    units: &UnitMap,
    fan_in: &HashMap<&str, usize>,
) -> serde_json::Value {
    let th = config.thresholds.by_name();
    // function_size is two-sided: the low side applies only to single-caller
    // (fan_in == 1) units; everything else is evaluated against the high side.
    let size = &config.thresholds.function_size;
    let size_bucket = |id: &str, v: usize| {
        if fan_in.get(id).copied().unwrap_or(0) == 1 {
            size.evaluate(v as f64)
        } else {
            size.high.evaluate(v as f64)
        }
    };
    serde_json::json!({
        "size": {
            "max": complexity.size.max,
            "mean": complexity.size.mean,
            "p90": complexity.size.p90,
            "distribution": distribution_json(&complexity.size.distribution, "id", "lines", units, size_bucket),
        },
        "params": {
            "max": complexity.params.max,
            "mean": complexity.params.mean,
            "distribution": distribution_json(&complexity.params.distribution, "id", "count", units, |_, v| th["params"].evaluate(v as f64)),
        },
        "cyclomatic": {
            "max": complexity.cyclomatic.max,
            "mean": complexity.cyclomatic.mean,
            "p90": complexity.cyclomatic.p90,
            "distribution": distribution_json(&complexity.cyclomatic.distribution, "id", "complexity", units, |_, v| th["cyclomatic"].evaluate(v as f64)),
        },
        "cognitive": {
            "max": complexity.cognitive.max,
            "mean": complexity.cognitive.mean,
            "p90": complexity.cognitive.p90,
            "distribution": distribution_json(&complexity.cognitive.distribution, "id", "complexity", units, |_, v| th["cognitive"].evaluate(v as f64)),
        },
        "max_scope": {
            "max": complexity.max_scope.max,
            "mean": complexity.max_scope.mean,
            "p90": complexity.max_scope.p90,
            "distribution": distribution_json(&complexity.max_scope.distribution, "id", "lines", units, |_, v| th["max_scope"].evaluate(v as f64)),
        },
    })
}

/// Build JSON for struct metrics with distributions
pub fn build_struct_json(
    struct_metrics: &StructMetrics,
    config: &Config,
    units: &UnitMap,
) -> serde_json::Value {
    let th = config.thresholds.by_name();
    serde_json::json!({
        "methods_per_struct": {
            "max": struct_metrics.methods_per_struct.max,
            "mean": struct_metrics.methods_per_struct.mean,
            "p90": struct_metrics.methods_per_struct.p90,
            "distribution": distribution_json(&struct_metrics.methods_per_struct.distribution, "id", "count", units, |_, v| th["methods_per_struct"].evaluate(v as f64)),
        },
        "lcom": {
            "max": struct_metrics.lcom.max,
            "mean": struct_metrics.lcom.mean,
            "distribution": distribution_json(&struct_metrics.lcom.distribution, "id", "lcom4", units, |_, v| th["lcom"].evaluate(v as f64)),
        },
    })
}

/// Build JSON for coverage metrics. `uncov_branches` is omitted when the
/// input lcov had no BRDA records.
pub fn build_coverage_json(cov: &CoverageMetrics) -> serde_json::Value {
    let line_dist: Vec<_> = cov
        .line_cov
        .distribution
        .iter()
        .map(|(id, pct)| serde_json::json!({"id": id, "line_cov_pct": pct}))
        .collect();
    let mut out = serde_json::json!({
        "line_cov": {
            "max": cov.line_cov.max,
            "mean": cov.line_cov.mean,
            "p90": cov.line_cov.p90,
            "distribution": line_dist,
        },
        "has_branches": cov.has_branches,
        "units_analyzed": cov.units_analyzed,
        "units_without_data": cov.units_without_data,
    });
    if cov.has_branches {
        let br_dist: Vec<_> = cov
            .uncov_branches
            .distribution
            .iter()
            .map(|(id, n)| serde_json::json!({"id": id, "uncov_branches": n}))
            .collect();
        out["uncov_branches"] = serde_json::json!({
            "max": cov.uncov_branches.max,
            "mean": cov.uncov_branches.mean,
            "p90": cov.uncov_branches.p90,
            "distribution": br_dist,
        });
    }
    out
}

/// Build JSON for file_loc metrics with distribution. Entries are keyed by
/// file path (not a unit id), so they carry a `bucket` but no source span.
pub fn build_file_loc_json(
    file_loc: &FileLocMetrics,
    config: &Config,
    units: &UnitMap,
) -> serde_json::Value {
    let th = config.thresholds.by_name();
    let distribution = distribution_json(
        &file_loc.distribution,
        "file",
        "lines",
        units,
        |_, v| th["file_loc"].evaluate(v as f64),
    );

    serde_json::json!({
        "max": file_loc.max,
        "mean": file_loc.mean,
        "p90": file_loc.p90,
        "total": file_loc.total,
        "distribution": distribution,
    })
}
