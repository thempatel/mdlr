//! Text and JSON formatting for the `mdlr check` command. Consumes the
//! `ComputedMetrics` produced in [`crate::check`] and renders it, honoring
//! disabled metrics and per-symbol filtering.

use anyhow::Result;
use std::io::Write;

use crate::cache::CacheStore;
use crate::check::{CheckFilter, ComputedMetrics, ScopeInfo};
use crate::check_scope::describe_scope;
use crate::cli::OutputFormat;
use crate::config;
use crate::display_scope::DisplayScope;
use crate::json_output::{
    build_bucketed_json, build_complexity_json, build_fan_metrics_json,
    build_file_loc_json, build_struct_json,
};
use crate::metrics_rows::{
    MetricSpecs, MetricsBundle, RowSelection, collect_metric_rows,
};
use mdlr_metrics::{BucketedMetrics, Thresholds};

/// Bundle the computed metrics for row collection.
fn metrics_bundle(computed: &ComputedMetrics) -> MetricsBundle<'_> {
    MetricsBundle {
        structural: &computed.structural,
        complexity: &computed.complexity,
        struct_metrics: &computed.struct_metrics,
        file_loc: &computed.file_loc,
        duplication: &computed.duplication,
        coverage: computed.coverage.as_ref(),
    }
}

/// Everything `render` needs beyond the computed metrics and config.
pub(crate) struct RenderArgs<'a> {
    pub format: OutputFormat,
    pub k: i32,
    pub pretty: bool,
    pub entry_count: usize,
    pub filter: &'a CheckFilter,
    pub scope: Option<&'a DisplayScope>,
}

/// Render `check` results in the requested output format.
pub(crate) fn render(
    computed: &ComputedMetrics,
    config: &config::Config,
    args: &RenderArgs,
    store: &CacheStore,
) -> anyhow::Result<()> {
    let scope_info = describe_scope(args.filter, args.scope);
    match args.format {
        OutputFormat::Text => format_text_output(
            computed,
            config,
            &TextOptions {
                k: args.k,
                pretty: args.pretty,
                filter: args.filter,
                scope: &scope_info,
            },
            store,
        ),
        OutputFormat::Json => {
            format_json_output(computed, config, args, &scope_info)
        }
    }
}

/// How rows should be selected for this run's filter.
fn row_selection<'a>(filter: &'a CheckFilter, k: i32) -> RowSelection<'a> {
    match filter {
        CheckFilter::Symbol(s) => RowSelection::Symbol(s.as_str()),
        _ => RowSelection::Top(k),
    }
}

/// Presentation options for `format_text_output`.
struct TextOptions<'a> {
    k: i32,
    pretty: bool,
    filter: &'a CheckFilter,
    scope: &'a ScopeInfo,
}

/// Format and print text output
fn format_text_output(
    computed: &ComputedMetrics,
    config: &config::Config,
    opts: &TextOptions,
    store: &CacheStore,
) -> Result<()> {
    // Diff mode switches scope silently on git state; always say which scope
    // this run reported on.
    println!("scope: {}", opts.scope.description);

    let bundle = metrics_bundle(computed);
    let ignores = store.ignores().load_ignores().unwrap_or_default();
    let rows = collect_metric_rows(
        &bundle,
        config,
        row_selection(opts.filter, opts.k),
        &ignores,
    );

    if opts.pretty {
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
fn format_json_output(
    computed: &ComputedMetrics,
    config: &config::Config,
    args: &RenderArgs,
    scope: &ScopeInfo,
) -> Result<()> {
    // When filtering by symbol, output specific metrics for that symbol
    if let CheckFilter::Symbol(symbol_id) = args.filter {
        let output = build_symbol_json(computed, config, symbol_id);
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let partial_count =
        computed.graph.units.iter().filter(|u| u.partial).count();

    let output = serde_json::json!({
        "scope": {
            "mode": scope.mode,
            "description": scope.description,
        },
        "files": {
            "extracted": args.entry_count,
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
    // Bucket the structural summary with the user's configured thresholds.
    let t = &config.thresholds;
    let thresholds = Thresholds {
        dag_density: t.dag_density.clone(),
        fan_in_max: t.fan_in_max.clone(),
        fan_in_mean: t.fan_in_mean.clone(),
        fan_out_max: t.fan_out_max.clone(),
        fan_out_mean: t.fan_out_mean.clone(),
    };
    let bucketed =
        BucketedMetrics::from_metrics(&computed.structural, &thresholds);

    let duplication_json = serde_json::json!({
        "max": computed.duplication.max,
        "mean": computed.duplication.mean,
        "p90": computed.duplication.p90,
        "clone_count": computed.duplication.clone_count,
        "distribution": computed.duplication.distribution.iter()
            .map(|(unit, pct)| serde_json::json!({"unit": unit, "duplication_pct": pct}))
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

/// Look up a symbol's value in a distribution.
fn find_value(dist: &[(String, usize)], symbol_id: &str) -> Option<usize> {
    dist.iter().find(|(n, _)| n == symbol_id).map(|(_, v)| *v)
}

/// Build JSON output for a specific symbol. Reuses the [`MetricSpecs`]
/// registry so the symbol view and the text rows read the same
/// distributions and thresholds. Unlike text rows, every metric with a
/// value for the symbol is shown (no boring/hub filtering).
fn build_symbol_json(
    computed: &ComputedMetrics,
    config: &config::Config,
    symbol_id: &str,
) -> serde_json::Value {
    let bundle = metrics_bundle(computed);
    let specs = MetricSpecs::new(&bundle, config);
    let mut metrics = serde_json::Map::new();
    let mut insert = |name: &str, value: usize, bucket: config::Bucket| {
        metrics.insert(
            name.to_string(),
            serde_json::json!({ "value": value, "bucket": bucket.to_string() }),
        );
    };

    for spec in &specs.int_specs {
        if let Some(value) = find_value(spec.distribution, symbol_id) {
            insert(spec.name, value, spec.bucket_for(value));
        }
    }
    if let Some(spec) = &specs.fan_out_spec
        && let Some(value) = find_value(spec.distribution, symbol_id)
    {
        insert("fan_out", value, spec.thresholds.evaluate(value as f64));
    }
    if let Some(spec) = &specs.fan_in_spec
        && let Some(value) = find_value(spec.distribution, symbol_id)
    {
        insert("fan_in", value, spec.thresholds.evaluate(value as f64));
    }
    if let Some(spec) = &specs.function_size_spec
        && let Some(value) = find_value(spec.distribution, symbol_id)
    {
        insert("function_size", value, spec.bucket_for(symbol_id, value));
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
