use crate::graph::{Graph, UnitKind};
use std::collections::HashMap;

/// Complexity metrics for functions
#[derive(Debug, Clone)]
pub struct ComplexityMetrics {
    /// Function size in lines of code
    pub size: SizeMetrics,
    /// Parameter counts
    pub params: ParamMetrics,
    /// Cyclomatic complexity (branches + 1)
    pub cyclomatic: CyclomaticMetrics,
}

#[derive(Debug, Clone)]
pub struct SizeMetrics {
    pub max: usize,
    pub mean: f64,
    pub p90: usize,
    /// Functions sorted by size descending
    pub distribution: Vec<(String, usize)>,
}

#[derive(Debug, Clone)]
pub struct ParamMetrics {
    pub max: usize,
    pub mean: f64,
    /// Functions sorted by param count descending
    pub distribution: Vec<(String, usize)>,
}

#[derive(Debug, Clone)]
pub struct CyclomaticMetrics {
    pub max: usize,
    pub mean: f64,
    pub p90: usize,
    /// Functions sorted by complexity descending
    pub distribution: Vec<(String, usize)>,
}

impl ComplexityMetrics {
    /// Compute complexity metrics from a graph
    pub fn compute(graph: &Graph) -> Self {
        let mut sizes: HashMap<String, usize> = HashMap::new();
        let mut params: HashMap<String, usize> = HashMap::new();
        let mut cyclomatic: HashMap<String, usize> = HashMap::new();

        for unit in &graph.units {
            // Only compute complexity for functions
            if unit.kind != UnitKind::Function {
                continue;
            }

            // Size from span
            let size = unit.span.end_line.saturating_sub(unit.span.start_line) + 1;
            sizes.insert(unit.id.clone(), size);

            // Parameter count (from unit.params if available)
            params.insert(unit.id.clone(), unit.params);

            // Cyclomatic complexity (from unit.branches if available)
            // Cyclomatic = branches + 1
            cyclomatic.insert(unit.id.clone(), unit.branches + 1);
        }

        Self {
            size: SizeMetrics::from_counts(sizes),
            params: ParamMetrics::from_counts(params),
            cyclomatic: CyclomaticMetrics::from_counts(cyclomatic),
        }
    }

    /// Check if there are any functions to report on
    pub fn has_functions(&self) -> bool {
        !self.size.distribution.is_empty()
    }
}

impl SizeMetrics {
    fn from_counts(counts: HashMap<String, usize>) -> Self {
        if counts.is_empty() {
            return Self {
                max: 0,
                mean: 0.0,
                p90: 0,
                distribution: vec![],
            };
        }

        let max = counts.values().copied().max().unwrap_or(0);
        let sum: usize = counts.values().sum();
        let mean = sum as f64 / counts.len() as f64;

        let mut sorted_values: Vec<usize> = counts.values().copied().collect();
        sorted_values.sort();
        let p90_idx = (sorted_values.len() as f64 * 0.9).ceil() as usize - 1;
        let p90 = sorted_values.get(p90_idx).copied().unwrap_or(max);

        let mut distribution: Vec<_> = counts.into_iter().collect();
        distribution.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        Self {
            max,
            mean,
            p90,
            distribution,
        }
    }
}

impl ParamMetrics {
    fn from_counts(counts: HashMap<String, usize>) -> Self {
        if counts.is_empty() {
            return Self {
                max: 0,
                mean: 0.0,
                distribution: vec![],
            };
        }

        let max = counts.values().copied().max().unwrap_or(0);
        let sum: usize = counts.values().sum();
        let mean = sum as f64 / counts.len() as f64;

        let mut distribution: Vec<_> = counts.into_iter().collect();
        distribution.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        Self {
            max,
            mean,
            distribution,
        }
    }
}

impl CyclomaticMetrics {
    fn from_counts(counts: HashMap<String, usize>) -> Self {
        if counts.is_empty() {
            return Self {
                max: 0,
                mean: 0.0,
                p90: 0,
                distribution: vec![],
            };
        }

        let max = counts.values().copied().max().unwrap_or(0);
        let sum: usize = counts.values().sum();
        let mean = sum as f64 / counts.len() as f64;

        let mut sorted_values: Vec<usize> = counts.values().copied().collect();
        sorted_values.sort();
        let p90_idx = (sorted_values.len() as f64 * 0.9).ceil() as usize - 1;
        let p90 = sorted_values.get(p90_idx).copied().unwrap_or(max);

        let mut distribution: Vec<_> = counts.into_iter().collect();
        distribution.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        Self {
            max,
            mean,
            p90,
            distribution,
        }
    }
}

impl std::fmt::Display for ComplexityMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Complexity Metrics")?;
        writeln!(f, "==================")?;
        writeln!(f)?;

        writeln!(
            f,
            "Function Size: max={} lines, mean={:.1}, p90={}",
            self.size.max, self.size.mean, self.size.p90
        )?;
        writeln!(
            f,
            "Parameters:    max={}, mean={:.1}",
            self.params.max, self.params.mean
        )?;
        writeln!(
            f,
            "Cyclomatic:    max={}, mean={:.1}, p90={}",
            self.cyclomatic.max, self.cyclomatic.mean, self.cyclomatic.p90
        )?;

        // Show top complex functions (by cyclomatic complexity)
        let complex: Vec<_> = self
            .cyclomatic
            .distribution
            .iter()
            .filter(|(_, c)| *c > 1)
            .take(10)
            .collect();

        if !complex.is_empty() {
            writeln!(f)?;
            writeln!(f, "Most Complex Functions:")?;
            for (name, complexity) in complex {
                let size = self
                    .size
                    .distribution
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, s)| *s)
                    .unwrap_or(0);
                let params = self
                    .params
                    .distribution
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, p)| *p)
                    .unwrap_or(0);
                writeln!(
                    f,
                    "  {} (cc={}, lines={}, params={})",
                    name, complexity, size, params
                )?;
            }
        }

        // Show largest functions
        let large: Vec<_> = self
            .size
            .distribution
            .iter()
            .filter(|(_, s)| *s > 20)
            .take(10)
            .collect();

        if !large.is_empty() {
            writeln!(f)?;
            writeln!(f, "Largest Functions:")?;
            for (name, size) in large {
                writeln!(f, "  {} ({} lines)", name, size)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Span, Unit};
    use std::path::PathBuf;

    fn make_function(id: &str, start: usize, end: usize, params: usize, branches: usize) -> Unit {
        Unit {
            id: id.to_string(),
            kind: UnitKind::Function,
            file: PathBuf::from("test.rs"),
            span: Span {
                start_line: start,
                start_col: 0,
                end_line: end,
                end_col: 0,
            },
            reads: vec![],
            writes: vec![],
            calls: vec![],
            tags: vec![],
            params,
            branches,
        }
    }

    #[test]
    fn test_size_metrics() {
        let mut graph = Graph::new();
        graph.add_unit(make_function("small", 1, 5, 0, 0)); // 5 lines
        graph.add_unit(make_function("medium", 10, 30, 2, 3)); // 21 lines
        graph.add_unit(make_function("large", 40, 100, 5, 10)); // 61 lines

        let metrics = ComplexityMetrics::compute(&graph);

        assert_eq!(metrics.size.max, 61);
        assert_eq!(metrics.size.distribution[0].0, "large");
        assert_eq!(metrics.size.distribution[0].1, 61);
    }

    #[test]
    fn test_param_metrics() {
        let mut graph = Graph::new();
        graph.add_unit(make_function("no_params", 1, 5, 0, 0));
        graph.add_unit(make_function("some_params", 10, 15, 3, 0));
        graph.add_unit(make_function("many_params", 20, 25, 7, 0));

        let metrics = ComplexityMetrics::compute(&graph);

        assert_eq!(metrics.params.max, 7);
        assert_eq!(metrics.params.distribution[0].0, "many_params");
    }

    #[test]
    fn test_cyclomatic_metrics() {
        let mut graph = Graph::new();
        graph.add_unit(make_function("simple", 1, 5, 0, 0)); // cc=1
        graph.add_unit(make_function("branchy", 10, 30, 0, 5)); // cc=6
        graph.add_unit(make_function("complex", 40, 100, 0, 15)); // cc=16

        let metrics = ComplexityMetrics::compute(&graph);

        assert_eq!(metrics.cyclomatic.max, 16);
        assert_eq!(metrics.cyclomatic.distribution[0].0, "complex");
        assert_eq!(metrics.cyclomatic.distribution[0].1, 16);
    }
}
