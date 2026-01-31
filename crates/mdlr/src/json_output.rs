//! JSON output formatting for the CLI.

use mdlr_metrics::{
    BucketedFanMetrics, BucketedValue, ComplexityMetrics, FileLocMetrics,
    StructMetrics, TagMetrics,
};

/// Build JSON for a bucketed metric value
pub fn build_bucketed_json(metric: &BucketedValue) -> serde_json::Value {
    serde_json::json!({
        "value": metric.value,
        "bucket": metric.bucket,
    })
}

/// Build JSON for fan metrics (fan_in/fan_out with max/mean)
pub fn build_fan_metrics_json(
    metrics: &BucketedFanMetrics,
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
    })
}

/// Build JSON for complexity metrics
pub fn build_complexity_json(
    complexity: &ComplexityMetrics,
) -> serde_json::Value {
    serde_json::json!({
        "size": {
            "max": complexity.size.max,
            "mean": complexity.size.mean,
            "p90": complexity.size.p90,
        },
        "params": {
            "max": complexity.params.max,
            "mean": complexity.params.mean,
        },
        "cyclomatic": {
            "max": complexity.cyclomatic.max,
            "mean": complexity.cyclomatic.mean,
            "p90": complexity.cyclomatic.p90,
        },
    })
}

/// Build JSON for struct metrics
pub fn build_struct_json(struct_metrics: &StructMetrics) -> serde_json::Value {
    serde_json::json!({
        "methods_per_struct": {
            "max": struct_metrics.methods_per_struct.max,
            "mean": struct_metrics.methods_per_struct.mean,
            "p90": struct_metrics.methods_per_struct.p90,
        },
        "lcom": {
            "max": struct_metrics.lcom.max,
            "mean": struct_metrics.lcom.mean,
        },
    })
}

/// Build JSON for file_loc metrics
pub fn build_file_loc_json(file_loc: &FileLocMetrics) -> serde_json::Value {
    serde_json::json!({
        "max": file_loc.max,
        "mean": file_loc.mean,
        "p90": file_loc.p90,
        "total": file_loc.total,
    })
}

/// Build JSON for semantic tags metrics
pub fn build_semantic_tags_json(
    tag_metrics: &TagMetrics,
) -> serde_json::Value {
    let namespace_distribution: serde_json::Map<String, serde_json::Value> =
        tag_metrics
            .namespace_distribution
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
            .collect();

    let namespace_values: serde_json::Map<String, serde_json::Value> =
        tag_metrics
            .namespace_values
            .iter()
            .map(|(ns, values)| {
                let values_map: serde_json::Map<String, serde_json::Value> =
                    values
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                        .collect();
                (ns.clone(), serde_json::Value::Object(values_map))
            })
            .collect();

    serde_json::json!({
        "total_units": tag_metrics.total_units,
        "tagged_units": tag_metrics.tagged_units,
        "coverage": tag_metrics.tag_coverage,
        "by_namespace": namespace_distribution,
        "namespace_values": namespace_values,
        "conceptual": build_conceptual_json(tag_metrics),
    })
}

/// Build conceptual metrics JSON if present
fn build_conceptual_json(
    tag_metrics: &TagMetrics,
) -> Option<serde_json::Value> {
    tag_metrics.conceptual.as_ref().map(|c| {
        let scattering: Vec<_> = c
            .concept_scattering
            .iter()
            .map(|s| {
                serde_json::json!({
                    "tag": s.tag,
                    "unit_count": s.unit_count,
                    "file_count": s.file_count,
                    "scatter_ratio": s.scatter_ratio,
                })
            })
            .collect();

        let cross_concept_by_ns: serde_json::Map<String, serde_json::Value> = c
            .cross_concept_edges
            .by_namespace
            .iter()
            .map(|(ns, pairs)| {
                let pairs_json: Vec<_> = pairs
                    .iter()
                    .map(|(from, to, count)| {
                        serde_json::json!({
                            "from": from,
                            "to": to,
                            "count": count,
                        })
                    })
                    .collect();
                (ns.clone(), serde_json::json!(pairs_json))
            })
            .collect();

        serde_json::json!({
            "conceptual_fan_out": {
                "max": c.conceptual_fan_out.max,
                "mean": c.conceptual_fan_out.mean,
                "top": c.conceptual_fan_out.top.iter().map(|(id, count)| {
                    serde_json::json!({"id": id, "count": count})
                }).collect::<Vec<_>>(),
            },
            "concept_scattering": scattering,
            "cross_concept_edges": {
                "total_tagged_edges": c.cross_concept_edges.total_tagged_edges,
                "cross_concept_count": c.cross_concept_edges.cross_concept_count,
                "cross_concept_ratio": c.cross_concept_edges.cross_concept_ratio,
                "by_namespace": cross_concept_by_ns,
            },
        })
    })
}
