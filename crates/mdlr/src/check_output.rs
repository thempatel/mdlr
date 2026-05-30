//! Text and JSON formatting for the `mdlr check` command. Consumes the
//! `ComputedMetrics` produced in [`crate::check`] and renders it, honoring
//! disabled metrics and per-symbol filtering.

use anyhow::Result;
use std::io::Write;

use crate::cache::CacheStore;
use crate::check::{CheckFilter, ComputedMetrics};
use crate::config;
use crate::json_output::{
    build_bucketed_json, build_complexity_json, build_fan_metrics_json,
    build_file_loc_json, build_struct_json,
};
use crate::metrics_rows::{MetricsBundle, collect_metric_rows};
use mdlr_metrics::{BucketedMetrics, Thresholds};

/// Extract symbol filter string from CheckFilter
fn get_symbol_filter(filter: &CheckFilter) -> Option<&str> {
    match filter {
        CheckFilter::Symbol(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Format and print text output
pub(crate) fn format_text_output(
    computed: &ComputedMetrics,
    config: &config::Config,
    k: i32,
    pretty: bool,
    filter: &CheckFilter,
    store: &CacheStore,
) -> Result<()> {
    let bundle = MetricsBundle {
        structural: &computed.structural,
        complexity: &computed.complexity,
        struct_metrics: &computed.struct_metrics,
        file_loc: &computed.file_loc,
        duplication: &computed.duplication,
        coverage: computed.coverage.as_ref(),
    };
    let symbol_filter = get_symbol_filter(filter);
    let ignores = store.ignores().load_ignores().unwrap_or_default();
    let rows =
        collect_metric_rows(&bundle, config, k, symbol_filter, &ignores);

    if pretty {
        let mut tw = tabwriter::TabWriter::new(vec![]);
        writeln!(tw, "metric\tsymbol\tvalue\tbucket")?;
        for (metric, symbol, value, bucket) in &rows {
            writeln!(tw, "{}\t{}\t{}\t{}", metric, symbol, value, bucket)?;
        }
        tw.flush()?;
        print!("{}", String::from_utf8_lossy(&tw.into_inner()?));
    } else {
        println!("metric\tsymbol\tvalue\tbucket");
        for (metric, symbol, value, bucket) in &rows {
            println!("{}\t{}\t{}\t{}", metric, symbol, value, bucket);
        }
    }

    let partial_count =
        computed.graph.units.iter().filter(|u| u.partial).count();
    if partial_count > 0 {
        eprintln!(
            "warning: {} unit(s) have partial extraction (compilation errors prevented full analysis)",
            partial_count
        );
    }

    Ok(())
}

/// Format and print JSON output
pub(crate) fn format_json_output(
    computed: &ComputedMetrics,
    config: &config::Config,
    extracted_count: usize,
    filter: &CheckFilter,
) -> Result<()> {
    // When filtering by symbol, output specific metrics for that symbol
    if let CheckFilter::Symbol(symbol_id) = filter {
        let output = build_symbol_json(computed, config, symbol_id);
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let partial_count =
        computed.graph.units.iter().filter(|u| u.partial).count();

    let output = serde_json::json!({
        "files": {
            "extracted": extracted_count,
        },
        "units": computed.graph.units.len(),
        "partial_units": partial_count,
        "edges": computed.graph.edges.len(),
        "metrics": build_metrics_json(computed, config),
    });
    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}

/// Assemble the full `metrics` JSON object, then drop any disabled metrics.
fn build_metrics_json(
    computed: &ComputedMetrics,
    config: &config::Config,
) -> serde_json::Value {
    let thresholds = Thresholds::default();
    let bucketed =
        BucketedMetrics::from_metrics(&computed.structural, &thresholds);

    let duplication_json = serde_json::json!({
        "max": computed.duplication.max,
        "mean": computed.duplication.mean,
        "p90": computed.duplication.p90,
        "clone_count": computed.duplication.clone_count,
        "distribution": computed.duplication.distribution.iter()
            .map(|(file, pct)| serde_json::json!({"file": file, "duplication_pct": pct}))
            .collect::<Vec<_>>(),
    });

    let mut metrics = serde_json::json!({
        "dag_density": build_bucketed_json(&bucketed.dag_density),
        "fan_in": build_fan_metrics_json(&bucketed.fan_in, &computed.structural.fan_in.distribution),
        "fan_out": build_fan_metrics_json(&bucketed.fan_out, &computed.structural.fan_out.distribution),
        "complexity": build_complexity_json(&computed.complexity),
        "struct": build_struct_json(&computed.struct_metrics),
        "file_loc": build_file_loc_json(&computed.file_loc),
        "duplication": duplication_json,
    });
    if let Some(cov) = computed.coverage.as_ref() {
        metrics["coverage"] = crate::json_output::build_coverage_json(cov);
    }

    prune_disabled_metrics(metrics.as_object_mut().expect("object"), config);
    metrics
}

/// Remove disabled metrics from the assembled `metrics` object: top-level keys
/// outright, and individual fields inside the composite `complexity`/`struct`/
/// `coverage` objects (dropping a composite entirely once it is emptied).
fn prune_disabled_metrics(
    metrics: &mut serde_json::Map<String, serde_json::Value>,
    config: &config::Config,
) {
    // (metric name, top-level JSON key)
    const TOP_LEVEL: &[(&str, &str)] = &[
        ("dag_density", "dag_density"),
        ("fan_in", "fan_in"),
        ("fan_out", "fan_out"),
        ("file_loc", "file_loc"),
        ("duplication_pct", "duplication"),
    ];
    // (metric name, composite parent key, sub-field key)
    const NESTED: &[(&str, &str, &str)] = &[
        ("function_size", "complexity", "size"),
        ("params", "complexity", "params"),
        ("cyclomatic", "complexity", "cyclomatic"),
        ("max_scope", "complexity", "max_scope"),
        ("methods_per_struct", "struct", "methods_per_struct"),
        ("lcom", "struct", "lcom"),
        ("line_cov", "coverage", "line_cov"),
        ("uncov_branches", "coverage", "uncov_branches"),
    ];

    for (metric, key) in TOP_LEVEL {
        if config.is_disabled(metric) {
            metrics.remove(*key);
        }
    }
    for (metric, parent, sub) in NESTED {
        if config.is_disabled(metric) {
            if let Some(obj) =
                metrics.get_mut(*parent).and_then(|v| v.as_object_mut())
            {
                obj.remove(*sub);
            }
        }
    }
    for parent in ["complexity", "struct", "coverage"] {
        let empty = metrics
            .get(parent)
            .and_then(|v| v.as_object())
            .is_some_and(|o| o.is_empty());
        if empty {
            metrics.remove(parent);
        }
    }
}

/// Insert a metric entry for a symbol if found in the distribution.
fn insert_symbol_metric(
    metrics: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    distribution: &[(String, usize)],
    thresholds: &config::MetricThresholds,
    symbol_id: &str,
    direction: mdlr_metrics::SortDirection,
) {
    if let Some((_, value)) = distribution.iter().find(|(n, _)| n == symbol_id)
    {
        let bucket = match direction {
            mdlr_metrics::SortDirection::Desc => {
                thresholds.evaluate(*value as f64)
            }
            mdlr_metrics::SortDirection::Asc => {
                thresholds.evaluate_asc(*value as f64)
            }
        };
        metrics.insert(
            name.to_string(),
            serde_json::json!({ "value": value, "bucket": bucket.to_string() }),
        );
    }
}

/// Build JSON output for a specific symbol
fn build_symbol_json(
    computed: &ComputedMetrics,
    config: &config::Config,
    symbol_id: &str,
) -> serde_json::Value {
    let mut metrics = serde_json::Map::new();
    let t = &config.thresholds;

    use mdlr_metrics::SortDirection::{Asc, Desc};
    let mut metric_sources: Vec<(
        &str,
        &[(String, usize)],
        &config::MetricThresholds,
        mdlr_metrics::SortDirection,
    )> = vec![
        (
            "fan_in",
            &computed.structural.fan_in.distribution,
            &t.fan_in_max,
            Desc,
        ),
        (
            "fan_out",
            &computed.structural.fan_out.distribution,
            &t.fan_out_max,
            Desc,
        ),
        (
            "function_size",
            &computed.complexity.size.distribution,
            &t.function_size,
            Desc,
        ),
        ("params", &computed.complexity.params.distribution, &t.params, Desc),
        (
            "cyclomatic",
            &computed.complexity.cyclomatic.distribution,
            &t.cyclomatic,
            Desc,
        ),
        (
            "cognitive",
            &computed.complexity.cognitive.distribution,
            &t.cognitive,
            Desc,
        ),
        (
            "max_scope",
            &computed.complexity.max_scope.distribution,
            &t.max_scope,
            Desc,
        ),
        (
            "methods_per_struct",
            &computed.struct_metrics.methods_per_struct.distribution,
            &t.methods_per_struct,
            Desc,
        ),
        ("lcom", &computed.struct_metrics.lcom.distribution, &t.lcom, Desc),
        (
            "duplication_pct",
            &computed.duplication.distribution,
            &t.duplication_pct,
            Desc,
        ),
    ];
    if let Some(cov) = computed.coverage.as_ref() {
        metric_sources.push((
            "line_cov",
            &cov.line_cov.distribution,
            &t.line_cov,
            Asc,
        ));
        if cov.has_branches {
            metric_sources.push((
                "uncov_branches",
                &cov.uncov_branches.distribution,
                &t.uncov_branches,
                Desc,
            ));
        }
    }

    metric_sources.retain(|(name, ..)| !config.is_disabled(name));

    for (name, distribution, thresholds, direction) in &metric_sources {
        insert_symbol_metric(
            &mut metrics,
            name,
            distribution,
            thresholds,
            symbol_id,
            *direction,
        );
    }

    let is_partial =
        computed.graph.units.iter().any(|u| u.id == symbol_id && u.partial);

    let mut output = serde_json::json!({
        "symbol": symbol_id,
        "metrics": metrics
    });
    if is_partial {
        output["partial"] = serde_json::json!(true);
    }
    output
}
