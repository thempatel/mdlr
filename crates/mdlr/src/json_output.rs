//! JSON output formatting for the CLI.

use mdlr_metrics::{
    BucketedFanMetrics, BucketedValue, ComplexityMetrics, FileLocMetrics,
    StructMetrics,
};

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
) -> serde_json::Value {
    let distribution_json: Vec<_> = distribution
        .iter()
        .map(|(id, count)| serde_json::json!({"id": id, "count": count}))
        .collect();

    serde_json::json!({
        "max": {
            "value": metrics.max.value as usize,
            "bucket": metrics.max.bucket,
        },
        "mean": {
            "value": metrics.mean.value,
            "bucket": metrics.mean.bucket,
        },
        "distribution": distribution_json,
    })
}

/// Build JSON for complexity metrics with distributions
pub fn build_complexity_json(
    complexity: &ComplexityMetrics,
) -> serde_json::Value {
    let size_distribution: Vec<_> = complexity
        .size
        .distribution
        .iter()
        .map(|(id, lines)| serde_json::json!({"id": id, "lines": lines}))
        .collect();

    let params_distribution: Vec<_> = complexity
        .params
        .distribution
        .iter()
        .map(|(id, count)| serde_json::json!({"id": id, "count": count}))
        .collect();

    let cyclomatic_distribution: Vec<_> = complexity
        .cyclomatic
        .distribution
        .iter()
        .map(|(id, cc)| serde_json::json!({"id": id, "complexity": cc}))
        .collect();

    let max_scope_distribution: Vec<_> = complexity
        .max_scope
        .distribution
        .iter()
        .map(|(id, lines)| serde_json::json!({"id": id, "lines": lines}))
        .collect();

    serde_json::json!({
        "size": {
            "max": complexity.size.max,
            "mean": complexity.size.mean,
            "p90": complexity.size.p90,
            "distribution": size_distribution,
        },
        "params": {
            "max": complexity.params.max,
            "mean": complexity.params.mean,
            "distribution": params_distribution,
        },
        "cyclomatic": {
            "max": complexity.cyclomatic.max,
            "mean": complexity.cyclomatic.mean,
            "p90": complexity.cyclomatic.p90,
            "distribution": cyclomatic_distribution,
        },
        "max_scope": {
            "max": complexity.max_scope.max,
            "mean": complexity.max_scope.mean,
            "p90": complexity.max_scope.p90,
            "distribution": max_scope_distribution,
        },
    })
}

/// Build JSON for struct metrics with distributions
pub fn build_struct_json(struct_metrics: &StructMetrics) -> serde_json::Value {
    let methods_distribution: Vec<_> = struct_metrics
        .methods_per_struct
        .distribution
        .iter()
        .map(|(id, count)| serde_json::json!({"id": id, "count": count}))
        .collect();

    let lcom_distribution: Vec<_> = struct_metrics
        .lcom
        .distribution
        .iter()
        .map(|(id, lcom4)| serde_json::json!({"id": id, "lcom4": lcom4}))
        .collect();

    serde_json::json!({
        "methods_per_struct": {
            "max": struct_metrics.methods_per_struct.max,
            "mean": struct_metrics.methods_per_struct.mean,
            "p90": struct_metrics.methods_per_struct.p90,
            "distribution": methods_distribution,
        },
        "lcom": {
            "max": struct_metrics.lcom.max,
            "mean": struct_metrics.lcom.mean,
            "distribution": lcom_distribution,
        },
    })
}

/// Build JSON for file_loc metrics with distribution
pub fn build_file_loc_json(file_loc: &FileLocMetrics) -> serde_json::Value {
    let distribution: Vec<_> = file_loc
        .distribution
        .iter()
        .map(|(file, lines)| serde_json::json!({"file": file, "lines": lines}))
        .collect();

    serde_json::json!({
        "max": file_loc.max,
        "mean": file_loc.mean,
        "p90": file_loc.p90,
        "total": file_loc.total,
        "distribution": distribution,
    })
}
