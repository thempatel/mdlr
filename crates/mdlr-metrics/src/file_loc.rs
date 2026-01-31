use mdlr_core::Graph;
use std::collections::HashMap;

/// Lines of code metrics for files
#[derive(Debug, Clone)]
pub struct FileLocMetrics {
    /// Maximum lines in any file
    pub max: usize,
    /// Mean lines per file
    pub mean: f64,
    /// 90th percentile of lines per file
    pub p90: usize,
    /// Total lines across all files
    pub total: usize,
    /// Files sorted by lines descending
    pub distribution: Vec<(String, usize)>,
}

impl FileLocMetrics {
    /// Compute file LOC metrics from a graph
    pub fn compute(graph: &Graph) -> Self {
        // Group units by file and find the max end_line per file
        let mut file_max_line: HashMap<String, usize> = HashMap::new();

        for unit in &graph.units {
            let file_path = unit.file.to_string_lossy().to_string();
            let entry = file_max_line.entry(file_path).or_insert(0);
            *entry = (*entry).max(unit.span.end_line);
        }

        Self::from_counts(file_max_line)
    }

    fn from_counts(counts: HashMap<String, usize>) -> Self {
        if counts.is_empty() {
            return Self {
                max: 0,
                mean: 0.0,
                p90: 0,
                total: 0,
                distribution: vec![],
            };
        }

        let max = counts.values().copied().max().unwrap_or(0);
        let total: usize = counts.values().sum();
        let mean = total as f64 / counts.len() as f64;

        let mut sorted_values: Vec<usize> = counts.values().copied().collect();
        sorted_values.sort();
        let p90_idx = (sorted_values.len() as f64 * 0.9).ceil() as usize - 1;
        let p90 = sorted_values.get(p90_idx).copied().unwrap_or(max);

        let mut distribution: Vec<_> = counts.into_iter().collect();
        distribution.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        Self { max, mean, p90, total, distribution }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdlr_core::{Graph, Span, Unit, UnitKind};
    use std::path::PathBuf;

    fn make_unit(file: &str, end_line: usize) -> Unit {
        Unit {
            id: format!("{}::unit_{}", file, end_line),
            kind: UnitKind::Function,
            file: PathBuf::from(file),
            span: Span { start_line: 1, start_col: 0, end_line, end_col: 0 },
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
    fn test_file_loc_metrics() {
        let mut graph = Graph::new();
        // File with 100 lines
        graph.add_unit(make_unit("src/small.rs", 50));
        graph.add_unit(make_unit("src/small.rs", 100));
        // File with 500 lines
        graph.add_unit(make_unit("src/medium.rs", 500));
        // File with 1000 lines
        graph.add_unit(make_unit("src/large.rs", 1000));

        let metrics = FileLocMetrics::compute(&graph);

        assert_eq!(metrics.max, 1000);
        assert_eq!(metrics.total, 1600); // 100 + 500 + 1000
        assert_eq!(metrics.distribution[0].0, "src/large.rs");
        assert_eq!(metrics.distribution[0].1, 1000);
    }

    #[test]
    fn test_empty_graph() {
        let graph = Graph::new();
        let metrics = FileLocMetrics::compute(&graph);

        assert_eq!(metrics.max, 0);
        assert_eq!(metrics.mean, 0.0);
        assert_eq!(metrics.total, 0);
        assert!(metrics.distribution.is_empty());
    }
}
