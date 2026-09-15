//! Display-time scope filtering for diff mode.
//!
//! Metrics are always computed over the full project graph (so values like
//! `fan_in` count callers everywhere); diff mode then narrows what is
//! *displayed* to the Units whose spans overlap changed lines, plus the
//! touched files for the file-keyed `file_loc` metric. After retaining the
//! in-scope entries, each distribution's aggregates (max/mean/p90) are
//! rebuilt so they describe the displayed scope.

use std::collections::HashSet;

use crate::check::ComputedMetrics;
use mdlr_metrics::{SortDirection, p90_boundary};

/// The set of Units and files one diff-mode `check` run reports on.
pub(crate) struct DisplayScope {
    /// Units whose span overlaps a changed line (all overlapping Units,
    /// parents included).
    pub unit_ids: HashSet<String>,
    /// `file_loc` keys (Unit `file` strings) of files with any changed line.
    pub files: HashSet<String>,
    /// Count of changed source files in scope, for the scope header.
    pub touched_files: usize,
}

/// Filter every distribution in `computed` down to the scope and rebuild
/// aggregates. Graph-global values (`dag_density`, coverage anomaly counters,
/// `clone_count`) are left project-wide.
pub(crate) fn apply(computed: &mut ComputedMetrics, scope: &DisplayScope) {
    let units = &scope.unit_ids;

    let s = &mut computed.structural;
    retain(&mut s.fan_in.distribution, units);
    (s.fan_in.max, s.fan_in.mean) = max_mean(&s.fan_in.distribution);
    retain(&mut s.fan_out.distribution, units);
    (s.fan_out.max, s.fan_out.mean) = max_mean(&s.fan_out.distribution);
    s.hubs.retain(|id, _| units.contains(id));

    let c = &mut computed.complexity;
    for dm in
        [&mut c.size, &mut c.cyclomatic, &mut c.cognitive, &mut c.max_scope]
    {
        dm.retain_ids(units);
    }
    retain(&mut c.params.distribution, units);
    (c.params.max, c.params.mean) = max_mean(&c.params.distribution);

    let st = &mut computed.struct_metrics;
    retain(&mut st.methods_per_struct.distribution, units);
    (st.methods_per_struct.max, st.methods_per_struct.mean) =
        max_mean(&st.methods_per_struct.distribution);
    st.methods_per_struct.p90 = p90(&st.methods_per_struct.distribution);
    retain(&mut st.lcom.distribution, units);
    (st.lcom.max, st.lcom.mean) = max_mean(&st.lcom.distribution);

    // `exclusive_fanout` is a gate signal, not a listing: it stays project-wide
    // so a helper whose parent is outside the diff still reads as exclusive.
    computed.inlined.inlined_size.retain_ids(units);

    let fl = &mut computed.file_loc;
    retain(&mut fl.distribution, &scope.files);
    (fl.max, fl.mean) = max_mean(&fl.distribution);
    fl.p90 = p90(&fl.distribution);
    fl.total = fl.distribution.iter().map(|(_, v)| v).sum();

    let d = &mut computed.duplication;
    retain(&mut d.distribution, units);
    // Duplication aggregates are f64 percentages; rebuild from the retained
    // rounded values (only units with any duplication appear).
    d.max = d.distribution.iter().map(|(_, v)| *v).max().unwrap_or(0) as f64;
    d.mean = if d.distribution.is_empty() {
        0.0
    } else {
        d.distribution.iter().map(|(_, v)| *v as f64).sum::<f64>()
            / d.distribution.len() as f64
    };
    d.p90 = p90(&d.distribution) as f64;

    if let Some(cov) = computed.coverage.as_mut() {
        cov.line_cov.retain_ids(units);
        cov.uncov_branches.retain_ids(units);
    }
}

fn retain(dist: &mut Vec<(String, usize)>, keep: &HashSet<String>) {
    dist.retain(|(id, _)| keep.contains(id));
}

fn max_mean(dist: &[(String, usize)]) -> (usize, f64) {
    if dist.is_empty() {
        return (0, 0.0);
    }
    let max = dist.iter().map(|(_, v)| *v).max().unwrap_or(0);
    let sum: usize = dist.iter().map(|(_, v)| v).sum();
    (max, sum as f64 / dist.len() as f64)
}

/// Worst-10% boundary for a higher-is-worse distribution.
fn p90(dist: &[(String, usize)]) -> usize {
    p90_boundary(dist.iter().map(|(_, v)| *v).collect(), SortDirection::Desc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(pairs: &[(&str, usize)]) -> Vec<(String, usize)> {
        pairs.iter().map(|(n, v)| (n.to_string(), *v)).collect()
    }

    #[test]
    fn retain_keeps_only_scope() {
        let mut d = dist(&[("a", 5), ("b", 3), ("c", 1)]);
        let keep: HashSet<String> =
            ["a".to_string(), "c".to_string()].into_iter().collect();
        retain(&mut d, &keep);
        assert_eq!(d, dist(&[("a", 5), ("c", 1)]));
    }

    #[test]
    fn max_mean_of_empty_is_zero() {
        assert_eq!(max_mean(&[]), (0, 0.0));
    }

    #[test]
    fn max_mean_recomputes() {
        let d = dist(&[("a", 10), ("b", 2)]);
        assert_eq!(max_mean(&d), (10, 6.0));
    }

    #[test]
    fn p90_desc_matches_from_counts_semantics() {
        // 10 values 1..=10: ceil(10*0.9)=9 → index 8 → value 9.
        let d: Vec<(String, usize)> =
            (1..=10).map(|i| (format!("u{i}"), i)).collect();
        assert_eq!(p90(&d), 9);
        // Asc: ceil(10*0.1)=1 → index 0 → value 1.
        let values: Vec<usize> = d.iter().map(|(_, v)| *v).collect();
        assert_eq!(p90_boundary(values, SortDirection::Asc), 1);
    }
}
