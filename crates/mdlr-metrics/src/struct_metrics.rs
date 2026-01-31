use mdlr_core::{Graph, UnitKind};
use std::collections::{HashMap, HashSet};

/// Metrics for structs (class-like constructs)
#[derive(Debug, Clone)]
pub struct StructMetrics {
    /// Methods per struct
    pub methods_per_struct: MethodsPerStruct,
    /// LCOM (Lack of Cohesion of Methods) per struct
    pub lcom: LcomMetrics,
}

#[derive(Debug, Clone)]
pub struct MethodsPerStruct {
    pub max: usize,
    pub mean: f64,
    pub p90: usize,
    /// Structs sorted by method count descending
    pub distribution: Vec<(String, usize)>,
}

#[derive(Debug, Clone)]
pub struct LcomMetrics {
    pub max: f64,
    pub mean: f64,
    /// Structs sorted by LCOM descending (higher = less cohesive)
    pub distribution: Vec<(String, f64)>,
}

impl StructMetrics {
    pub fn compute(graph: &Graph) -> Self {
        let methods_per_struct = compute_methods_per_struct(graph);
        let lcom = compute_lcom(graph);

        Self { methods_per_struct, lcom }
    }

    pub fn has_structs(&self) -> bool {
        !self.methods_per_struct.distribution.is_empty()
    }
}

fn compute_methods_per_struct(graph: &Graph) -> MethodsPerStruct {
    let mut struct_method_count: HashMap<String, usize> = HashMap::new();

    // Initialize all structs with 0 methods
    for unit in &graph.units {
        if unit.kind == UnitKind::Struct {
            struct_method_count.insert(unit.id.clone(), 0);
        }
    }

    // Count methods per struct (methods have parent pointing to struct)
    for unit in &graph.units {
        if unit.kind == UnitKind::Method {
            if let Some(ref parent) = unit.parent {
                *struct_method_count.entry(parent.clone()).or_insert(0) += 1;
            }
        }
    }

    if struct_method_count.is_empty() {
        return MethodsPerStruct {
            max: 0,
            mean: 0.0,
            p90: 0,
            distribution: vec![],
        };
    }

    let max = struct_method_count.values().copied().max().unwrap_or(0);
    let sum: usize = struct_method_count.values().sum();
    let mean = sum as f64 / struct_method_count.len() as f64;

    let mut sorted_values: Vec<usize> =
        struct_method_count.values().copied().collect();
    sorted_values.sort();
    let p90_idx = (sorted_values.len() as f64 * 0.9).ceil() as usize - 1;
    let p90 = sorted_values.get(p90_idx).copied().unwrap_or(max);

    let mut distribution: Vec<_> = struct_method_count.into_iter().collect();
    distribution.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    MethodsPerStruct { max, mean, p90, distribution }
}

/// Compute LCOM for a single struct.
/// Returns the normalized LCOM value (0.0 = cohesive, 1.0 = incohesive).
fn compute_struct_lcom(methods: &[&mdlr_core::Unit]) -> f64 {
    if methods.len() < 2 {
        return 0.0;
    }

    // Count pairs of methods that share fields vs don't share
    let mut shares_field = 0;
    let mut no_shared_field = 0;

    for i in 0..methods.len() {
        for j in (i + 1)..methods.len() {
            let fields_i: HashSet<_> = methods[i]
                .reads
                .iter()
                .chain(methods[i].writes.iter())
                .collect();
            let fields_j: HashSet<_> = methods[j]
                .reads
                .iter()
                .chain(methods[j].writes.iter())
                .collect();

            if fields_i.intersection(&fields_j).next().is_some() {
                shares_field += 1;
            } else {
                no_shared_field += 1;
            }
        }
    }

    // LCOM = max(0, P - Q) where P = pairs not sharing, Q = pairs sharing
    let lcom = if no_shared_field > shares_field {
        (no_shared_field - shares_field) as f64
    } else {
        0.0
    };

    // Normalize by total pairs for comparison
    let total_pairs = (methods.len() * (methods.len() - 1)) / 2;
    if total_pairs > 0 { lcom / total_pairs as f64 } else { 0.0 }
}

fn compute_lcom(graph: &Graph) -> LcomMetrics {
    // Group methods by struct (parent)
    let mut struct_methods: HashMap<String, Vec<&mdlr_core::Unit>> =
        HashMap::new();

    for unit in &graph.units {
        if unit.kind == UnitKind::Method {
            if let Some(ref parent) = unit.parent {
                struct_methods.entry(parent.clone()).or_default().push(unit);
            }
        }
    }

    let lcom_values: HashMap<String, f64> = struct_methods
        .into_iter()
        .map(|(struct_id, methods)| (struct_id, compute_struct_lcom(&methods)))
        .collect();

    if lcom_values.is_empty() {
        return LcomMetrics { max: 0.0, mean: 0.0, distribution: vec![] };
    }

    let max = lcom_values
        .values()
        .copied()
        .fold(0.0, |a, b| if b > a { b } else { a });
    let sum: f64 = lcom_values.values().sum();
    let mean = sum / lcom_values.len() as f64;

    let mut distribution: Vec<_> = lcom_values.into_iter().collect();
    distribution.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    LcomMetrics { max, mean, distribution }
}

impl std::fmt::Display for StructMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Struct Metrics")?;
        writeln!(f, "==============")?;
        writeln!(f)?;

        writeln!(
            f,
            "Methods/Struct: max={}, mean={:.1}, p90={}",
            self.methods_per_struct.max,
            self.methods_per_struct.mean,
            self.methods_per_struct.p90
        )?;

        writeln!(
            f,
            "LCOM:           max={:.2}, mean={:.2}",
            self.lcom.max, self.lcom.mean
        )?;

        // Show largest structs
        let large: Vec<_> = self
            .methods_per_struct
            .distribution
            .iter()
            .filter(|(_, c)| *c > 5)
            .take(10)
            .collect();

        if !large.is_empty() {
            writeln!(f)?;
            writeln!(f, "Largest Structs:")?;
            for (name, count) in large {
                writeln!(f, "  {} ({} methods)", name, count)?;
            }
        }

        // Show least cohesive structs
        let incohesive: Vec<_> = self
            .lcom
            .distribution
            .iter()
            .filter(|(_, lcom)| *lcom > 0.5)
            .take(10)
            .collect();

        if !incohesive.is_empty() {
            writeln!(f)?;
            writeln!(f, "Least Cohesive Structs (high LCOM):")?;
            for (name, lcom) in incohesive {
                writeln!(f, "  {} (LCOM={:.2})", name, lcom)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdlr_core::{Span, Unit};
    use std::path::PathBuf;

    fn make_struct(id: &str) -> Unit {
        Unit {
            id: id.to_string(),
            kind: UnitKind::Struct,
            file: PathBuf::from("test.rs"),
            span: Span {
                start_line: 1,
                start_col: 0,
                end_line: 10,
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

    fn make_method(
        id: &str,
        parent: &str,
        reads: Vec<&str>,
        writes: Vec<&str>,
    ) -> Unit {
        Unit {
            id: id.to_string(),
            kind: UnitKind::Method,
            file: PathBuf::from("test.rs"),
            span: Span {
                start_line: 1,
                start_col: 0,
                end_line: 5,
                end_col: 0,
            },
            reads: reads.into_iter().map(|s| s.to_string()).collect(),
            writes: writes.into_iter().map(|s| s.to_string()).collect(),
            calls: vec![],
            tags: vec![],
            params: 0,
            branches: 0,
            parent: Some(parent.to_string()),
        }
    }

    #[test]
    fn test_methods_per_struct() {
        let mut graph = Graph::new();
        graph.add_unit(make_struct("Foo"));
        graph.add_unit(make_struct("Bar"));
        graph.add_unit(make_method("Foo::new", "Foo", vec![], vec![]));
        graph.add_unit(make_method("Foo::get", "Foo", vec![], vec![]));
        graph.add_unit(make_method("Foo::set", "Foo", vec![], vec![]));
        graph.add_unit(make_method("Bar::run", "Bar", vec![], vec![]));

        let metrics = StructMetrics::compute(&graph);

        assert_eq!(metrics.methods_per_struct.max, 3);
        assert_eq!(metrics.methods_per_struct.distribution[0].0, "Foo");
        assert_eq!(metrics.methods_per_struct.distribution[0].1, 3);
    }

    #[test]
    fn test_lcom_cohesive() {
        let mut graph = Graph::new();
        graph.add_unit(make_struct("Foo"));
        // Both methods access the same field - cohesive
        graph.add_unit(make_method("Foo::get_x", "Foo", vec!["x"], vec![]));
        graph.add_unit(make_method("Foo::set_x", "Foo", vec![], vec!["x"]));

        let metrics = StructMetrics::compute(&graph);

        // Methods share field access, so LCOM should be 0
        assert_eq!(metrics.lcom.max, 0.0);
    }

    #[test]
    fn test_lcom_incohesive() {
        let mut graph = Graph::new();
        graph.add_unit(make_struct("Foo"));
        // Methods access different fields - incohesive
        graph.add_unit(make_method("Foo::get_x", "Foo", vec!["x"], vec![]));
        graph.add_unit(make_method("Foo::get_y", "Foo", vec!["y"], vec![]));
        graph.add_unit(make_method("Foo::get_z", "Foo", vec!["z"], vec![]));

        let metrics = StructMetrics::compute(&graph);

        // No methods share field access, so LCOM should be > 0
        assert!(metrics.lcom.max > 0.0);
    }
}
