//! Metric row collection for CLI output.

use crate::cache::Ignores;
use crate::config::{Bucket, Config, MetricThresholds, TwoSidedThresholds};
use mdlr_cpd::DuplicationMetrics;
use mdlr_metrics::{
    ComplexityMetrics, CoverageMetrics, FileLocMetrics, HubInfo,
    SortDirection, StructMetrics, StructuralMetrics,
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
    /// Build a row for `symbol`/`value`, bucketed by a higher-is-worse
    /// threshold table. Shared by the threshold-gated specs (fan_in, fan_out).
    fn bucketed(
        metric_name: &str,
        symbol: &str,
        value: usize,
        thresholds: &MetricThresholds,
    ) -> Self {
        ScoredRow {
            metric_name: metric_name.to_string(),
            symbol: symbol.to_string(),
            value: value.to_string(),
            bucket: thresholds.evaluate(value as f64),
        }
    }

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
    pub duplication: &'a DuplicationMetrics,
    /// Present iff the user passed `--cov`.
    pub coverage: Option<&'a CoverageMetrics>,
}

/// Walk a distribution, optionally restricted to one symbol, pushing
/// whatever rows `score` produces. The single collection loop shared by
/// every spec type and both display modes (global and symbol-filtered).
fn collect_rows(
    distribution: &[(String, usize)],
    filter: Option<&str>,
    rows: &mut Vec<ScoredRow>,
    score: impl Fn(&str, usize) -> Option<ScoredRow>,
) {
    for (name, value) in distribution {
        if filter.is_none_or(|f| name == f)
            && let Some(row) = score(name, *value)
        {
            rows.push(row);
        }
    }
}

/// Specification for collecting an integer metric.
///
/// `direction` controls both the threshold evaluation (which end of the
/// range is "worse") and which side of `boring_threshold` to keep:
/// - `Desc`: keep entries with `value > boring_threshold` (boring = small).
/// - `Asc`:  keep entries with `value < boring_threshold` (boring = large).
pub(crate) struct IntMetricSpec<'a> {
    pub(crate) name: &'static str,
    pub(crate) distribution: &'a [(String, usize)],
    thresholds: MetricThresholds,
    boring_threshold: usize,
    direction: SortDirection,
}

impl IntMetricSpec<'_> {
    fn is_interesting(&self, value: usize) -> bool {
        match self.direction {
            SortDirection::Desc => value > self.boring_threshold,
            SortDirection::Asc => value < self.boring_threshold,
        }
    }

    pub(crate) fn bucket_for(&self, value: usize) -> Bucket {
        match self.direction {
            SortDirection::Desc => self.thresholds.evaluate(value as f64),
            SortDirection::Asc => self.thresholds.evaluate_asc(value as f64),
        }
    }

    fn format_value(&self, value: usize) -> String {
        match self.name {
            "line_cov" => format!("{value}%"),
            _ => value.to_string(),
        }
    }

    fn score(&self, symbol: &str, value: usize) -> Option<ScoredRow> {
        self.is_interesting(value).then(|| ScoredRow {
            metric_name: self.name.to_string(),
            symbol: symbol.to_string(),
            value: self.format_value(value),
            bucket: self.bucket_for(value),
        })
    }
}

/// Specification for collecting `function_size`, the only two-sided metric:
/// both extremes are bad. The high side always applies; the low side applies
/// only to units with exactly one visible caller (`fan_in == 1`) — the
/// single-caller pass-through case where "inline into the caller" is
/// well-defined. `fan_in == 0` (callers unknown to the graph: trait dispatch,
/// pub API, entry points) and `fan_in >= 2` (shared helpers) are exempt and
/// evaluated against the high side only.
pub(crate) struct TwoSidedSizeSpec<'a> {
    pub(crate) distribution: &'a [(String, usize)],
    thresholds: &'a TwoSidedThresholds,
    fan_in: HashMap<&'a str, usize>,
}

impl<'a> TwoSidedSizeSpec<'a> {
    fn new(
        m: &'a MetricsBundle,
        thresholds: &'a TwoSidedThresholds,
    ) -> TwoSidedSizeSpec<'a> {
        TwoSidedSizeSpec {
            distribution: &m.complexity.size.distribution,
            thresholds,
            fan_in: m
                .structural
                .fan_in
                .distribution
                .iter()
                .map(|(id, v)| (id.as_str(), *v))
                .collect(),
        }
    }

    fn low_side_applies(&self, symbol: &str) -> bool {
        self.fan_in.get(symbol).copied().unwrap_or(0) == 1
    }

    pub(crate) fn bucket_for(&self, symbol: &str, value: usize) -> Bucket {
        if self.low_side_applies(symbol) {
            self.thresholds.evaluate(value as f64)
        } else {
            self.thresholds.high.evaluate(value as f64)
        }
    }

    fn score(&self, symbol: &str, value: usize) -> Option<ScoredRow> {
        // Boring = 1-liners that are exempt from the low side (the high-side
        // `value > 1` rule that applied before the metric became two-sided).
        if value <= 1 && !self.low_side_applies(symbol) {
            return None;
        }
        Some(ScoredRow {
            metric_name: "function_size".to_string(),
            symbol: symbol.to_string(),
            value: value.to_string(),
            bucket: self.bucket_for(symbol, value),
        })
    }
}

/// Specification for collecting `fan_out` with Delegator filtering.
///
/// A unit is a *Delegator* when its high `fan_out` is accompanied by low
/// internal complexity — both `cyclomatic` and `cognitive` sit below their
/// `fair` thresholds. Such a unit just forwards work to many callees, so a
/// high `fan_out` is usually good design rather than a refactoring target;
/// its row is suppressed in global/top-k output. In symbol-filter mode the
/// value is always shown (mirrors [`HubFilteredFanInSpec`]).
///
/// The complexity lookups read the computed `cyclomatic`/`cognitive`
/// distributions directly, so the gate works even when those metrics are in
/// `disabled_metrics` (disabling is output-control, not compute-control). A
/// symbol absent from a distribution is treated as 0 — i.e. low.
pub(crate) struct DelegatorFilteredFanOutSpec<'a> {
    pub(crate) distribution: &'a [(String, usize)],
    pub(crate) thresholds: MetricThresholds,
    cyclomatic: HashMap<&'a str, usize>,
    cognitive: HashMap<&'a str, usize>,
    cyclomatic_fair: f64,
    cognitive_fair: f64,
}

impl<'a> DelegatorFilteredFanOutSpec<'a> {
    fn new(
        m: &'a MetricsBundle,
        thresholds: MetricThresholds,
        th: &HashMap<String, MetricThresholds>,
    ) -> Self {
        let by_symbol = |dist: &'a [(String, usize)]| {
            dist.iter().map(|(id, v)| (id.as_str(), *v)).collect()
        };
        DelegatorFilteredFanOutSpec {
            distribution: &m.structural.fan_out.distribution,
            thresholds,
            cyclomatic: by_symbol(&m.complexity.cyclomatic.distribution),
            cognitive: by_symbol(&m.complexity.cognitive.distribution),
            cyclomatic_fair: th["cyclomatic"].fair,
            cognitive_fair: th["cognitive"].fair,
        }
    }

    fn is_delegator(&self, symbol: &str) -> bool {
        let cyc = self.cyclomatic.get(symbol).copied().unwrap_or(0);
        let cog = self.cognitive.get(symbol).copied().unwrap_or(0);
        (cyc as f64) < self.cyclomatic_fair
            && (cog as f64) < self.cognitive_fair
    }

    /// In global mode (`require_non_delegator`) Delegators are dropped; in
    /// symbol-filter mode the value is shown regardless. `fan_out == 0`
    /// units are always boring and produce no row.
    fn score(
        &self,
        symbol: &str,
        value: usize,
        require_non_delegator: bool,
    ) -> Option<ScoredRow> {
        if value == 0 {
            return None;
        }
        if require_non_delegator && self.is_delegator(symbol) {
            return None;
        }
        Some(ScoredRow::bucketed("fan_out", symbol, value, &self.thresholds))
    }
}

/// Specification for collecting `cyclomatic` with Dispatcher filtering.
///
/// A unit is a *Dispatcher* when its high `cyclomatic` is breadth, not depth:
/// `cognitive` stays below its `fair` threshold (one `match`/`switch` arm per
/// enum or AST variant, shallow). Such a high branch count is a flat dispatch
/// rather than a refactoring target, so its row is suppressed in global/top-k
/// output. In symbol-filter mode the value is always shown (mirrors
/// [`DelegatorFilteredFanOutSpec`]). The gate never fires once `cognitive` is at
/// `fair` or worse, so genuinely-nested units stay visible.
///
/// The `cognitive` lookup reads the computed distribution directly, so the gate
/// works even when `cognitive` is in `disabled_metrics` (disabling is
/// output-control, not compute-control). A symbol absent from the distribution
/// is treated as 0 — i.e. low, a Dispatcher.
pub(crate) struct DispatcherFilteredCyclomaticSpec<'a> {
    pub(crate) distribution: &'a [(String, usize)],
    pub(crate) thresholds: MetricThresholds,
    cognitive: HashMap<&'a str, usize>,
    cognitive_fair: f64,
}

impl<'a> DispatcherFilteredCyclomaticSpec<'a> {
    fn new(
        m: &'a MetricsBundle,
        thresholds: MetricThresholds,
        th: &HashMap<String, MetricThresholds>,
    ) -> Self {
        DispatcherFilteredCyclomaticSpec {
            distribution: &m.complexity.cyclomatic.distribution,
            thresholds,
            cognitive: m
                .complexity
                .cognitive
                .distribution
                .iter()
                .map(|(id, v)| (id.as_str(), *v))
                .collect(),
            cognitive_fair: th["cognitive"].fair,
        }
    }

    fn is_dispatcher(&self, symbol: &str) -> bool {
        let cog = self.cognitive.get(symbol).copied().unwrap_or(0);
        (cog as f64) < self.cognitive_fair
    }

    /// In global mode (`require_non_dispatcher`) Dispatchers are dropped; in
    /// symbol-filter mode the value is shown regardless. `cyclomatic <= 1`
    /// units are boring (a single linear path) and produce no row.
    fn score(
        &self,
        symbol: &str,
        value: usize,
        require_non_dispatcher: bool,
    ) -> Option<ScoredRow> {
        if value <= 1 {
            return None;
        }
        if require_non_dispatcher && self.is_dispatcher(symbol) {
            return None;
        }
        Some(ScoredRow::bucketed(
            "cyclomatic",
            symbol,
            value,
            &self.thresholds,
        ))
    }
}

/// Specification for collecting `params` with wide-signature filtering.
///
/// A unit has a *wide signature* when its high `params` count is breadth of
/// passive inputs, not a behavioral-knob explosion: both `cyclomatic` and
/// `cognitive` sit below their `fair` thresholds, so the parameters are
/// threaded context handles, injected dependencies, CLI flags, or
/// object-construction inputs rather than independent control inputs. Such a
/// long signature is usually appropriate rather than a refactoring target, so
/// its row is suppressed in global/top-k output. In symbol-filter mode the
/// value is always shown (mirrors [`DelegatorFilteredFanOutSpec`]).
///
/// The complexity lookups read the computed `cyclomatic`/`cognitive`
/// distributions directly, so the gate works even when those metrics are in
/// `disabled_metrics` (disabling is output-control, not compute-control). A
/// symbol absent from a distribution is treated as 0 — i.e. low.
pub(crate) struct WideSignatureFilteredParamsSpec<'a> {
    pub(crate) distribution: &'a [(String, usize)],
    pub(crate) thresholds: MetricThresholds,
    cyclomatic: HashMap<&'a str, usize>,
    cognitive: HashMap<&'a str, usize>,
    cyclomatic_fair: f64,
    cognitive_fair: f64,
}

impl<'a> WideSignatureFilteredParamsSpec<'a> {
    fn new(
        m: &'a MetricsBundle,
        thresholds: MetricThresholds,
        th: &HashMap<String, MetricThresholds>,
    ) -> Self {
        let by_symbol = |dist: &'a [(String, usize)]| {
            dist.iter().map(|(id, v)| (id.as_str(), *v)).collect()
        };
        WideSignatureFilteredParamsSpec {
            distribution: &m.complexity.params.distribution,
            thresholds,
            cyclomatic: by_symbol(&m.complexity.cyclomatic.distribution),
            cognitive: by_symbol(&m.complexity.cognitive.distribution),
            cyclomatic_fair: th["cyclomatic"].fair,
            cognitive_fair: th["cognitive"].fair,
        }
    }

    fn is_wide_signature(&self, symbol: &str) -> bool {
        let cyc = self.cyclomatic.get(symbol).copied().unwrap_or(0);
        let cog = self.cognitive.get(symbol).copied().unwrap_or(0);
        (cyc as f64) < self.cyclomatic_fair
            && (cog as f64) < self.cognitive_fair
    }

    /// In global mode (`require_non_wide`) wide-signature units are dropped; in
    /// symbol-filter mode the value is shown regardless. `params == 0` units
    /// are always boring and produce no row.
    fn score(
        &self,
        symbol: &str,
        value: usize,
        require_non_wide: bool,
    ) -> Option<ScoredRow> {
        if value == 0 {
            return None;
        }
        if require_non_wide && self.is_wide_signature(symbol) {
            return None;
        }
        Some(ScoredRow::bucketed("params", symbol, value, &self.thresholds))
    }
}

/// Specification for collecting fan_in metric with hub filtering
/// Only includes units that are hubs (high fan_in AND high fan_out)
pub(crate) struct HubFilteredFanInSpec<'a> {
    pub(crate) distribution: &'a [(String, usize)],
    pub(crate) thresholds: MetricThresholds,
    hubs: &'a HashMap<String, HubInfo>,
}

impl HubFilteredFanInSpec<'_> {
    /// In global mode only hub units are shown (`require_hub`); in symbol
    /// filter mode the value is always shown regardless of hub status.
    fn score(
        &self,
        symbol: &str,
        value: usize,
        require_hub: bool,
    ) -> Option<ScoredRow> {
        if require_hub && !self.hubs.contains_key(symbol) {
            return None;
        }
        Some(ScoredRow::bucketed("fan_in", symbol, value, &self.thresholds))
    }
}

/// The line_cov / uncov_branches specs, present only when `--cov` was
/// passed. Skip 100% covered: boring 100 with Asc keeps everything below
/// 100% in, so a unit at exactly 100% drops out.
fn coverage_specs<'a>(
    cov: &'a CoverageMetrics,
    th: &HashMap<String, MetricThresholds>,
) -> Vec<IntMetricSpec<'a>> {
    let mut specs = vec![IntMetricSpec {
        name: "line_cov",
        distribution: &cov.line_cov.distribution,
        thresholds: th["line_cov"].clone(),
        boring_threshold: 100,
        direction: SortDirection::Asc,
    }];
    if cov.has_branches {
        specs.push(IntMetricSpec {
            name: "uncov_branches",
            distribution: &cov.uncov_branches.distribution,
            thresholds: th["uncov_branches"].clone(),
            boring_threshold: 0,
            direction: SortDirection::Desc,
        });
    }
    specs
}

/// Bundled metric specifications for collection. The single registry of
/// which distributions and thresholds each metric reads, shared by the text
/// rows and the symbol JSON view.
pub(crate) struct MetricSpecs<'a> {
    pub(crate) int_specs: Vec<IntMetricSpec<'a>>,
    pub(crate) function_size_spec: Option<TwoSidedSizeSpec<'a>>,
    pub(crate) fan_in_spec: Option<HubFilteredFanInSpec<'a>>,
    pub(crate) fan_out_spec: Option<DelegatorFilteredFanOutSpec<'a>>,
    pub(crate) cyclomatic_spec: Option<DispatcherFilteredCyclomaticSpec<'a>>,
    pub(crate) params_spec: Option<WideSignatureFilteredParamsSpec<'a>>,
}

impl<'a> MetricSpecs<'a> {
    pub(crate) fn new(m: &'a MetricsBundle, config: &'a Config) -> Self {
        // Thresholds resolve by metric name (config serde keys).
        let th = config.thresholds.by_name();
        let c = m.complexity;
        let spec = |name: &'static str,
                    distribution: &'a [(String, usize)],
                    boring_threshold: usize| {
            IntMetricSpec {
                name,
                distribution,
                thresholds: th[name].clone(),
                boring_threshold,
                direction: SortDirection::Desc,
            }
        };
        let mut int_specs = vec![
            spec("cognitive", &c.cognitive.distribution, 1),
            spec("max_scope", &c.max_scope.distribution, 0),
            spec(
                "methods_per_struct",
                &m.struct_metrics.methods_per_struct.distribution,
                0,
            ),
            spec("file_loc", &m.file_loc.distribution, 0),
            spec("duplication_pct", &m.duplication.distribution, 0),
            spec("lcom", &m.struct_metrics.lcom.distribution, 0),
        ];
        if let Some(cov) = m.coverage {
            int_specs.extend(coverage_specs(cov, &th));
        }
        // Disabling is output-control: drop specs so no view shows them.
        int_specs.retain(|spec| !config.is_disabled(spec.name));

        let function_size_spec =
            (!config.is_disabled("function_size")).then(|| {
                TwoSidedSizeSpec::new(m, &config.thresholds.function_size)
            });
        let fan_in_spec =
            (!config.is_disabled("fan_in")).then(|| HubFilteredFanInSpec {
                distribution: &m.structural.fan_in.distribution,
                thresholds: th["fan_in"].clone(),
                hubs: &m.structural.hubs,
            });
        let fan_out_spec = (!config.is_disabled("fan_out")).then(|| {
            DelegatorFilteredFanOutSpec::new(m, th["fan_out"].clone(), &th)
        });
        let cyclomatic_spec = (!config.is_disabled("cyclomatic")).then(|| {
            DispatcherFilteredCyclomaticSpec::new(
                m,
                th["cyclomatic"].clone(),
                &th,
            )
        });
        let params_spec = (!config.is_disabled("params")).then(|| {
            WideSignatureFilteredParamsSpec::new(m, th["params"].clone(), &th)
        });

        MetricSpecs {
            int_specs,
            function_size_spec,
            fan_in_spec,
            fan_out_spec,
            cyclomatic_spec,
            params_spec,
        }
    }

    /// Collect rows from every spec. `filter` restricts to one symbol
    /// (symbol view); `None` collects everything (global sorting mode).
    fn collect(&self, filter: Option<&str>) -> Vec<ScoredRow> {
        let mut rows = Vec::new();
        if let Some(spec) = &self.fan_out_spec {
            collect_rows(spec.distribution, filter, &mut rows, |s, v| {
                spec.score(s, v, filter.is_none())
            });
        }
        if let Some(spec) = &self.cyclomatic_spec {
            collect_rows(spec.distribution, filter, &mut rows, |s, v| {
                spec.score(s, v, filter.is_none())
            });
        }
        if let Some(spec) = &self.params_spec {
            collect_rows(spec.distribution, filter, &mut rows, |s, v| {
                spec.score(s, v, filter.is_none())
            });
        }
        for spec in &self.int_specs {
            collect_rows(spec.distribution, filter, &mut rows, |s, v| {
                spec.score(s, v)
            });
        }
        if let Some(spec) = &self.function_size_spec {
            collect_rows(spec.distribution, filter, &mut rows, |s, v| {
                spec.score(s, v)
            });
        }
        if let Some(spec) = &self.fan_in_spec {
            collect_rows(spec.distribution, filter, &mut rows, |s, v| {
                spec.score(s, v, filter.is_none())
            });
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
    "cognitive",
    "max_scope",
    "methods_per_struct",
    "file_loc",
    "duplication_pct",
    "lcom",
    "line_cov",
    "uncov_branches",
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

/// Which rows `collect_metric_rows` returns.
pub enum RowSelection<'a> {
    /// One symbol's rows, unsorted and unlimited.
    Symbol(&'a str),
    /// Top-k rows across all metrics, prioritized by severity (critical
    /// first, then poor, fair, good, excellent) and grouped by metric for
    /// display. Negative k collects all rows.
    Top(i32),
}

/// Collect metric rows for text output.
pub fn collect_metric_rows(
    metrics: &MetricsBundle,
    config: &Config,
    selection: RowSelection,
    ignores: &Ignores,
) -> Vec<MetricRow> {
    let specs = MetricSpecs::new(metrics, config);
    let filter = match selection {
        RowSelection::Symbol(s) => Some(s),
        RowSelection::Top(_) => None,
    };

    let mut rows = specs.collect(filter);
    rows.retain(|row| !ignores.is_ignored(&row.symbol, &row.metric_name));

    match selection {
        RowSelection::Symbol(_) => {
            rows.into_iter().map(ScoredRow::into_row).collect()
        }
        RowSelection::Top(k) => sort_and_group(rows, k),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_with<'a>(
        distribution: &'a [(String, usize)],
        thresholds: &'a TwoSidedThresholds,
        fan_in: &[(&'a str, usize)],
    ) -> TwoSidedSizeSpec<'a> {
        TwoSidedSizeSpec {
            distribution,
            thresholds,
            fan_in: fan_in.iter().copied().collect(),
        }
    }

    #[test]
    fn low_side_flags_only_single_caller_units() {
        let thresholds = Config::default().thresholds.function_size;
        let distribution = vec![
            ("pass_through".to_string(), 1),
            ("trait_impl".to_string(), 1),
            ("shared_getter".to_string(), 1),
            ("small_single_caller".to_string(), 3),
            ("big".to_string(), 250),
        ];
        let spec = spec_with(
            &distribution,
            &thresholds,
            &[
                ("pass_through", 1),
                // trait_impl absent from fan_in map -> fan_in 0, exempt
                ("shared_getter", 30),
                ("small_single_caller", 1),
                ("big", 1),
            ],
        );

        let mut rows = Vec::new();
        collect_rows(spec.distribution, None, &mut rows, |s, v| {
            spec.score(s, v)
        });
        let by_symbol: HashMap<&str, Bucket> =
            rows.iter().map(|r| (r.symbol.as_str(), r.bucket)).collect();

        // fan_in == 1: low side applies.
        assert_eq!(by_symbol["pass_through"], Bucket::Poor);
        assert_eq!(by_symbol["small_single_caller"], Bucket::Fair);
        // Exempt 1-liners are boring and produce no row at all.
        assert!(!by_symbol.contains_key("trait_impl"));
        assert!(!by_symbol.contains_key("shared_getter"));
        // The high side is unaffected by the gate.
        assert_eq!(by_symbol["big"], Bucket::Critical);
    }

    fn fanout_spec<'a>(
        distribution: &'a [(String, usize)],
        cyclomatic: &[(&'a str, usize)],
        cognitive: &[(&'a str, usize)],
    ) -> DelegatorFilteredFanOutSpec<'a> {
        let th = Config::default().thresholds.by_name();
        DelegatorFilteredFanOutSpec {
            distribution,
            thresholds: th["fan_out"].clone(),
            cyclomatic: cyclomatic.iter().copied().collect(),
            cognitive: cognitive.iter().copied().collect(),
            cyclomatic_fair: th["cyclomatic"].fair,
            cognitive_fair: th["cognitive"].fair,
        }
    }

    #[test]
    fn delegator_fanout_suppressed_in_global_only() {
        let distribution = vec![
            ("delegator".to_string(), 40),
            ("branchy".to_string(), 40),
            ("nested".to_string(), 40),
        ];
        let spec = fanout_spec(
            &distribution,
            // cyclomatic: delegator low, branchy high, nested low
            &[("delegator", 2), ("branchy", 25), ("nested", 2)],
            // cognitive: delegator low, branchy low, nested high
            &[("delegator", 1), ("branchy", 1), ("nested", 20)],
        );

        // Global mode: a high-fan_out unit with BOTH complexities low is a
        // Delegator and is suppressed; either complexity being high flags it.
        assert!(spec.score("delegator", 40, true).is_none());
        assert!(spec.score("branchy", 40, true).is_some());
        assert!(spec.score("nested", 40, true).is_some());

        // A symbol absent from the complexity maps reads as 0 -> low -> a
        // Delegator -> suppressed.
        assert!(spec.score("unknown", 40, true).is_none());

        // Symbol-filter mode is exempt: the Delegator's value is still shown,
        // with its real (Critical) bucket.
        let row = spec.score("delegator", 40, false).unwrap();
        assert_eq!(row.bucket, Bucket::Critical);

        // fan_out == 0 is always boring, in either mode.
        assert!(spec.score("delegator", 0, true).is_none());
        assert!(spec.score("delegator", 0, false).is_none());
    }

    fn cyclomatic_spec<'a>(
        distribution: &'a [(String, usize)],
        cognitive: &[(&'a str, usize)],
    ) -> DispatcherFilteredCyclomaticSpec<'a> {
        let th = Config::default().thresholds.by_name();
        DispatcherFilteredCyclomaticSpec {
            distribution,
            thresholds: th["cyclomatic"].clone(),
            cognitive: cognitive.iter().copied().collect(),
            cognitive_fair: th["cognitive"].fair,
        }
    }

    #[test]
    fn dispatcher_cyclomatic_suppressed_in_global_only() {
        let distribution =
            vec![("dispatcher".to_string(), 30), ("nested".to_string(), 30)];
        // cognitive: dispatcher low (flat breadth), nested high (real depth).
        let spec = cyclomatic_spec(
            &distribution,
            &[("dispatcher", 2), ("nested", 25)],
        );

        // Global mode: high cyclomatic with low cognitive is a Dispatcher and
        // is suppressed; high cognitive (real nesting) keeps it visible.
        assert!(spec.score("dispatcher", 30, true).is_none());
        assert!(spec.score("nested", 30, true).is_some());

        // A symbol absent from the cognitive map reads as 0 -> low -> a
        // Dispatcher -> suppressed.
        assert!(spec.score("unknown", 30, true).is_none());

        // Symbol-filter mode is exempt: the Dispatcher's value is still shown,
        // with its real (Critical) bucket.
        let row = spec.score("dispatcher", 30, false).unwrap();
        assert_eq!(row.bucket, Bucket::Critical);

        // cyclomatic <= 1 is always boring (single linear path), either mode.
        assert!(spec.score("dispatcher", 1, true).is_none());
        assert!(spec.score("nested", 1, false).is_none());
    }

    fn params_spec<'a>(
        distribution: &'a [(String, usize)],
        cyclomatic: &[(&'a str, usize)],
        cognitive: &[(&'a str, usize)],
    ) -> WideSignatureFilteredParamsSpec<'a> {
        let th = Config::default().thresholds.by_name();
        WideSignatureFilteredParamsSpec {
            distribution,
            thresholds: th["params"].clone(),
            cyclomatic: cyclomatic.iter().copied().collect(),
            cognitive: cognitive.iter().copied().collect(),
            cyclomatic_fair: th["cyclomatic"].fair,
            cognitive_fair: th["cognitive"].fair,
        }
    }

    #[test]
    fn wide_signature_params_suppressed_in_global_only() {
        let distribution = vec![
            ("wide".to_string(), 8),
            ("branchy".to_string(), 8),
            ("nested".to_string(), 8),
        ];
        let spec = params_spec(
            &distribution,
            // cyclomatic: wide low, branchy high, nested low
            &[("wide", 2), ("branchy", 25), ("nested", 2)],
            // cognitive: wide low, branchy low, nested high
            &[("wide", 1), ("branchy", 1), ("nested", 20)],
        );

        // Global mode: a many-param unit with BOTH complexities low is a wide
        // signature (passive inputs) and is suppressed; either complexity being
        // high flags it as a genuine god-function.
        assert!(spec.score("wide", 8, true).is_none());
        assert!(spec.score("branchy", 8, true).is_some());
        assert!(spec.score("nested", 8, true).is_some());

        // A symbol absent from the complexity maps reads as 0 -> low -> a wide
        // signature -> suppressed.
        assert!(spec.score("unknown", 8, true).is_none());

        // Symbol-filter mode is exempt: the value is still shown, with its real
        // (Poor) bucket.
        let row = spec.score("wide", 8, false).unwrap();
        assert_eq!(row.bucket, Bucket::Poor);

        // params == 0 is always boring, in either mode.
        assert!(spec.score("wide", 0, true).is_none());
        assert!(spec.score("wide", 0, false).is_none());
    }
}
