use serde::{Deserialize, Serialize};

/// Canonical bucket and per-metric threshold types live in `mdlr-metrics`;
/// re-exported here so config consumers have one import path.
pub use mdlr_metrics::{Bucket, MetricThresholds};

/// Display mode for metric output
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    #[default]
    Both,
    Label,
    Value,
}

/// Thresholds for a two-sided metric — one where both extremes are bad.
/// `low` is evaluated lower-is-worse, `high` higher-is-worse; a value gets
/// the worse of the two buckets. The ideal range sits between
/// `low.excellent` and `high.excellent`.
///
/// Deserializes from either the split `{low: {...}, high: {...}}` form or
/// the old flat `{excellent, good, fair, poor}` form, which is treated as
/// the high side with the default low side.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "TwoSidedRepr")]
pub struct TwoSidedThresholds {
    pub low: MetricThresholds,
    pub high: MetricThresholds,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TwoSidedRepr {
    Flat(MetricThresholds),
    Split(SplitRepr),
}

/// `deny_unknown_fields` keeps a malformed flat form (e.g. a lone
/// `excellent: 20`) from silently matching as an empty split form.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SplitRepr {
    low: Option<MetricThresholds>,
    high: Option<MetricThresholds>,
}

impl From<TwoSidedRepr> for TwoSidedThresholds {
    fn from(repr: TwoSidedRepr) -> Self {
        let d = DEFAULT_FUNCTION_SIZE;
        match repr {
            TwoSidedRepr::Flat(high) => Self { low: d.low, high },
            TwoSidedRepr::Split(s) => Self {
                low: s.low.unwrap_or(d.low),
                high: s.high.unwrap_or(d.high),
            },
        }
    }
}

impl TwoSidedThresholds {
    /// Evaluate against both sides, taking the worse bucket. Callers that
    /// exempt a unit from the low side use `self.high.evaluate()` directly.
    pub fn evaluate(&self, value: f64) -> Bucket {
        let low = self.low.evaluate_asc(value);
        let high = self.high.evaluate(value);
        if (low as u8) > (high as u8) { low } else { high }
    }
}

/// Display configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    #[serde(default)]
    pub mode: DisplayMode,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self { mode: DisplayMode::Both }
    }
}

/// All thresholds configuration. Container-level `#[serde(default)]` fills any
/// missing field from the `Default` impl below, so a config may override just
/// the metrics it cares about.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThresholdsConfig {
    pub dag_density: MetricThresholds,
    pub fan_in_max: MetricThresholds,
    pub fan_in_mean: MetricThresholds,
    pub fan_out_max: MetricThresholds,
    pub fan_out_mean: MetricThresholds,
    pub function_size: TwoSidedThresholds,
    pub params: MetricThresholds,
    pub cyclomatic: MetricThresholds,
    pub cognitive: MetricThresholds,
    pub methods_per_struct: MetricThresholds,
    pub lcom: MetricThresholds,
    pub file_loc: MetricThresholds,
    pub max_scope: MetricThresholds,
    pub duplication_pct: MetricThresholds,
    pub line_cov: MetricThresholds,
    pub uncov_branches: MetricThresholds,
}

/// Shorthand constructor for default threshold tables.
const fn mt(
    excellent: f64,
    good: f64,
    fair: f64,
    poor: f64,
) -> MetricThresholds {
    MetricThresholds { excellent, good, fair, poor }
}

/// Default `function_size` thresholds. Two-sided: tiny functions are flagged
/// too, but only when fan_in == 1 (single-caller pass-throughs). The low
/// `poor: 1` keeps critical unreachable on the low side — a 1-liner never
/// outranks a god function.
const DEFAULT_FUNCTION_SIZE: TwoSidedThresholds = TwoSidedThresholds {
    low: mt(5.0, 4.0, 3.0, 1.0),
    high: mt(20.0, 50.0, 100.0, 200.0),
};

/// Default thresholds, tuned on empirical observations of healthy codebases.
const DEFAULT_THRESHOLDS: ThresholdsConfig = ThresholdsConfig {
    dag_density: mt(0.5, 1.0, 1.5, 2.0),
    fan_in_max: mt(3.0, 5.0, 10.0, 15.0),
    fan_in_mean: mt(0.5, 1.0, 2.0, 3.0),
    fan_out_max: mt(3.0, 5.0, 8.0, 12.0),
    fan_out_mean: mt(0.5, 1.0, 2.0, 3.0),
    function_size: DEFAULT_FUNCTION_SIZE,
    params: mt(3.0, 5.0, 7.0, 10.0),
    cyclomatic: mt(5.0, 10.0, 20.0, 30.0),
    cognitive: mt(5.0, 10.0, 15.0, 25.0),
    methods_per_struct: mt(5.0, 10.0, 15.0, 25.0),
    // LCOM4 = connected components. 1 = cohesive, 2+ = should split
    lcom: mt(2.0, 3.0, 4.0, 5.0),
    file_loc: mt(200.0, 400.0, 600.0, 1000.0),
    max_scope: mt(15.0, 30.0, 50.0, 100.0),
    duplication_pct: mt(3.0, 5.0, 10.0, 20.0),
    // Lower-is-worse: fields are the LOW boundary of each bucket.
    line_cov: mt(90.0, 80.0, 70.0, 60.0),
    uncov_branches: mt(1.0, 3.0, 6.0, 10.0),
};

impl Default for ThresholdsConfig {
    fn default() -> Self {
        DEFAULT_THRESHOLDS
    }
}

impl ThresholdsConfig {
    /// Threshold tables keyed by canonical metric name, derived from the
    /// config's own serde keys so there is no separate name→field registry
    /// to maintain. `fan_in`/`fan_out` map to their `*_max` thresholds.
    /// `function_size` is two-sided and not included; read
    /// `self.function_size.low/high` directly.
    pub fn by_name(
        &self,
    ) -> std::collections::HashMap<String, MetricThresholds> {
        let serde_json::Value::Object(map) =
            serde_json::to_value(self).expect("thresholds serialize")
        else {
            unreachable!("ThresholdsConfig serializes to an object");
        };
        map.into_iter()
            .filter_map(|(key, v)| {
                let name = match key.as_str() {
                    "fan_in_max" => "fan_in".to_string(),
                    "fan_out_max" => "fan_out".to_string(),
                    _ => key,
                };
                // function_size's two-sided shape fails this parse and
                // drops out, as intended.
                Some((name, serde_json::from_value(v).ok()?))
            })
            .collect()
    }

    /// Get thresholds for a single metric by canonical name.
    pub fn get(&self, name: &str) -> Option<MetricThresholds> {
        self.by_name().remove(name)
    }
}

/// Hub detection thresholds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubThresholds {
    /// Minimum fan_in to be considered a hub candidate (default: 10)
    pub min_fan_in: usize,
    /// Minimum fan_out to be considered a hub (default: 3)
    pub min_fan_out: usize,
}

impl Default for HubThresholds {
    fn default() -> Self {
        Self {
            min_fan_in: mdlr_metrics::DEFAULT_HUB_MIN_FAN_IN,
            min_fan_out: mdlr_metrics::DEFAULT_HUB_MIN_FAN_OUT,
        }
    }
}

/// CPD (Copy-Paste Detection) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpdConfig {
    /// Minimum number of tokens for a block to be considered a duplicate (default: 50)
    pub min_tokens: usize,
}

impl Default for CpdConfig {
    fn default() -> Self {
        Self { min_tokens: 50 }
    }
}

/// Canonical metric names — the identifiers shown by `mdlr metrics ls`,
/// printed in the `metric` column / JSON keys, and accepted in
/// `disabled_metrics`. The single source of truth for which names are valid.
pub const METRIC_NAMES: &[&str] = &[
    "dag_density",
    "fan_in",
    "fan_out",
    "function_size",
    "params",
    "cyclomatic",
    "cognitive",
    "max_scope",
    "methods_per_struct",
    "lcom",
    "file_loc",
    "duplication_pct",
    "line_cov",
    "uncov_branches",
];

/// Main configuration struct
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub thresholds: ThresholdsConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub hub: HubThresholds,
    #[serde(default)]
    pub cpd: CpdConfig,
    /// Canonical metric names to suppress from `check` output. Disabling is an
    /// output-control concern: bundled compute passes still run, but the CPD
    /// and coverage passes are skipped when their metrics are fully disabled.
    #[serde(default)]
    pub disabled_metrics: Vec<String>,
}

impl Config {
    /// Whether a metric (by canonical name) is disabled in this config.
    pub fn is_disabled(&self, metric: &str) -> bool {
        self.disabled_metrics.iter().any(|m| m == metric)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_threshold_evaluation() {
        let thresholds = MetricThresholds {
            excellent: 0.5,
            good: 1.0,
            fair: 1.5,
            poor: 2.0,
        };

        assert_eq!(thresholds.evaluate(0.3), Bucket::Excellent);
        assert_eq!(thresholds.evaluate(0.5), Bucket::Good);
        assert_eq!(thresholds.evaluate(0.7), Bucket::Good);
        assert_eq!(thresholds.evaluate(1.0), Bucket::Fair);
        assert_eq!(thresholds.evaluate(1.3), Bucket::Fair);
        assert_eq!(thresholds.evaluate(1.5), Bucket::Poor);
        assert_eq!(thresholds.evaluate(1.8), Bucket::Poor);
        assert_eq!(thresholds.evaluate(2.0), Bucket::Critical);
        assert_eq!(thresholds.evaluate(5.0), Bucket::Critical);
    }

    #[test]
    fn two_sided_evaluate_takes_worse_bucket() {
        let t = DEFAULT_FUNCTION_SIZE;
        // Low side: 5+ excellent, 4 good, 3 fair, <=2 poor (critical
        // unreachable since size >= 1).
        assert_eq!(t.evaluate(1.0), Bucket::Poor);
        assert_eq!(t.evaluate(2.0), Bucket::Poor);
        assert_eq!(t.evaluate(3.0), Bucket::Fair);
        assert_eq!(t.evaluate(4.0), Bucket::Good);
        assert_eq!(t.evaluate(5.0), Bucket::Excellent);
        // High side unchanged.
        assert_eq!(t.evaluate(19.0), Bucket::Excellent);
        assert_eq!(t.evaluate(50.0), Bucket::Fair);
        assert_eq!(t.evaluate(250.0), Bucket::Critical);
    }

    #[test]
    fn two_sided_parses_flat_form_as_high_side() {
        let t: TwoSidedThresholds = serde_yaml::from_str(
            "excellent: 10\ngood: 30\nfair: 60\npoor: 120\n",
        )
        .unwrap();
        assert_eq!(t.high.excellent, 10.0);
        assert_eq!(t.high.poor, 120.0);
        // Low side falls back to defaults.
        assert_eq!(t.low.excellent, 5.0);
        assert_eq!(t.low.poor, 1.0);
    }

    #[test]
    fn two_sided_parses_split_form() {
        let t: TwoSidedThresholds = serde_yaml::from_str(
            "low:\n  excellent: 6\n  good: 5\n  fair: 4\n  poor: 2\nhigh:\n  excellent: 25\n  good: 60\n  fair: 120\n  poor: 240\n",
        )
        .unwrap();
        assert_eq!(t.low.excellent, 6.0);
        assert_eq!(t.high.excellent, 25.0);
    }

    #[test]
    fn two_sided_split_form_allows_one_side() {
        let t: TwoSidedThresholds = serde_yaml::from_str(
            "low:\n  excellent: 6\n  good: 5\n  fair: 4\n  poor: 2\n",
        )
        .unwrap();
        assert_eq!(t.low.excellent, 6.0);
        // High side falls back to defaults.
        assert_eq!(t.high.excellent, 20.0);
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.display.mode, DisplayMode::Both);
        assert_eq!(config.thresholds.dag_density.excellent, 0.5);
        assert!(config.disabled_metrics.is_empty());
    }

    #[test]
    fn test_is_disabled() {
        let config = Config {
            disabled_metrics: vec!["lcom".to_string(), "fan_in".to_string()],
            ..Default::default()
        };
        assert!(config.is_disabled("lcom"));
        assert!(config.is_disabled("fan_in"));
        assert!(!config.is_disabled("cyclomatic"));
    }

    #[test]
    fn metric_names_cover_all_thresholds() {
        // Every threshold-backed metric name must be a recognized canonical
        // name so it can be disabled.
        for name in
            ["dag_density", "fan_in", "lcom", "duplication_pct", "line_cov"]
        {
            assert!(METRIC_NAMES.contains(&name), "missing {name}");
        }
    }
}
