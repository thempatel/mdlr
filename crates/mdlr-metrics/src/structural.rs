use mdlr_core::Graph;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct StructuralMetrics {
    pub dag_density: f64,
    pub fan_in: FanMetrics,
    pub fan_out: FanMetrics,
    /// Hub information for each unit (units with high fan_in AND high fan_out)
    pub hubs: HashMap<String, HubInfo>,
}

#[derive(Debug, Clone)]
pub struct FanMetrics {
    pub max: usize,
    pub mean: f64,
    pub distribution: Vec<(String, usize)>,
}

/// Hub information for a unit
#[derive(Debug, Clone)]
pub struct HubInfo {
    pub fan_in: usize,
    pub fan_out: usize,
}

impl FanMetrics {
    fn from_counts(counts: HashMap<String, usize>) -> Self {
        let max = counts.values().copied().max().unwrap_or(0);
        let sum: usize = counts.values().sum();
        let mean = if counts.is_empty() {
            0.0
        } else {
            sum as f64 / counts.len() as f64
        };

        let mut distribution: Vec<_> = counts.into_iter().collect();
        distribution.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        Self { max, mean, distribution }
    }
}

/// Default threshold for minimum fan_in to be considered a hub candidate
pub const DEFAULT_HUB_MIN_FAN_IN: usize = 10;
/// Default threshold for minimum fan_out to be considered a hub
pub const DEFAULT_HUB_MIN_FAN_OUT: usize = 3;

pub fn compute(graph: &Graph) -> StructuralMetrics {
    compute_with_hub_thresholds(
        graph,
        DEFAULT_HUB_MIN_FAN_IN,
        DEFAULT_HUB_MIN_FAN_OUT,
    )
}

#[tracing::instrument(name = "compute_structural", skip_all)]
pub fn compute_with_hub_thresholds(
    graph: &Graph,
    hub_min_fan_in: usize,
    hub_min_fan_out: usize,
) -> StructuralMetrics {
    let node_count = graph.units.len();
    let edge_count = graph.edges.len();

    let dag_density = if node_count > 1 {
        edge_count as f64 / (node_count - 1) as f64
    } else {
        0.0
    };

    let mut fan_out_counts: HashMap<String, usize> = HashMap::new();
    let mut fan_in_counts: HashMap<String, usize> = HashMap::new();

    for unit in &graph.units {
        fan_out_counts.insert(unit.id.clone(), 0);
        fan_in_counts.insert(unit.id.clone(), 0);
    }

    for edge in &graph.edges {
        *fan_out_counts.entry(edge.from.clone()).or_insert(0) += 1;
        *fan_in_counts.entry(edge.to.clone()).or_insert(0) += 1;
    }

    // Build hub info for units with high fan_in AND high fan_out
    let mut hubs: HashMap<String, HubInfo> = HashMap::new();
    for unit in &graph.units {
        let fan_in = *fan_in_counts.get(&unit.id).unwrap_or(&0);
        let fan_out = *fan_out_counts.get(&unit.id).unwrap_or(&0);

        if fan_in >= hub_min_fan_in && fan_out >= hub_min_fan_out {
            hubs.insert(unit.id.clone(), HubInfo { fan_in, fan_out });
        }
    }

    StructuralMetrics {
        dag_density,
        fan_in: FanMetrics::from_counts(fan_in_counts),
        fan_out: FanMetrics::from_counts(fan_out_counts),
        hubs,
    }
}

impl std::fmt::Display for StructuralMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Structural Metrics")?;
        writeln!(f, "==================")?;
        writeln!(f)?;
        writeln!(f, "DAG Density: {:.3}", self.dag_density)?;
        writeln!(f)?;
        writeln!(
            f,
            "Fan-In:  max={}, mean={:.2}",
            self.fan_in.max, self.fan_in.mean
        )?;
        writeln!(
            f,
            "Fan-Out: max={}, mean={:.2}",
            self.fan_out.max, self.fan_out.mean
        )?;

        if !self.fan_out.distribution.is_empty() {
            writeln!(f)?;
            writeln!(f, "Top Fan-Out:")?;
            for (name, count) in self.fan_out.distribution.iter().take(10) {
                if *count > 0 {
                    writeln!(f, "  {} ({})", name, count)?;
                }
            }
        }

        if !self.fan_in.distribution.is_empty() {
            writeln!(f)?;
            writeln!(f, "Top Fan-In:")?;
            for (name, count) in self.fan_in.distribution.iter().take(10) {
                if *count > 0 {
                    writeln!(f, "  {} ({})", name, count)?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdlr_core::{Edge, EdgeKind, Span, Unit, UnitKind};
    use std::path::PathBuf;

    fn make_unit(id: &str) -> Unit {
        Unit {
            id: id.to_string(),
            kind: UnitKind::Function,
            file: PathBuf::from("test.rs"),
            span: Span {
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 0,
            },
            reads: vec![],
            writes: vec![],
            calls: vec![],
            tags: vec![],
            params: 0,
            branches: 0,
            parent: None,
        }
    }

    #[test]
    fn test_empty_graph() {
        let graph = Graph::new();
        let metrics = compute(&graph);
        assert_eq!(metrics.dag_density, 0.0);
        assert_eq!(metrics.fan_in.max, 0);
        assert_eq!(metrics.fan_out.max, 0);
    }

    #[test]
    fn test_linear_chain() {
        let mut graph = Graph::new();
        graph.add_unit(make_unit("a"));
        graph.add_unit(make_unit("b"));
        graph.add_unit(make_unit("c"));
        graph.add_edge(Edge {
            from: "a".to_string(),
            to: "b".to_string(),
            kind: EdgeKind::Calls,
        });
        graph.add_edge(Edge {
            from: "b".to_string(),
            to: "c".to_string(),
            kind: EdgeKind::Calls,
        });

        let metrics = compute(&graph);
        assert_eq!(metrics.dag_density, 1.0);
        assert_eq!(metrics.fan_out.max, 1);
        assert_eq!(metrics.fan_in.max, 1);
    }

    #[test]
    fn test_star_topology() {
        let mut graph = Graph::new();
        graph.add_unit(make_unit("center"));
        graph.add_unit(make_unit("a"));
        graph.add_unit(make_unit("b"));
        graph.add_unit(make_unit("c"));

        for target in ["a", "b", "c"] {
            graph.add_edge(Edge {
                from: "center".to_string(),
                to: target.to_string(),
                kind: EdgeKind::Calls,
            });
        }

        let metrics = compute(&graph);
        assert_eq!(metrics.fan_out.max, 3);
        assert_eq!(metrics.fan_in.max, 1);
    }
}
