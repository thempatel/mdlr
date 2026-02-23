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
    pub max: usize,
    pub mean: f64,
    /// Structs sorted by LCOM4 descending (higher = more connected components = less cohesive)
    pub distribution: Vec<(String, usize)>,
}

impl StructMetrics {
    #[tracing::instrument(name = "compute_struct_metrics", skip_all)]
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

/// Compute LCOM4 for a single struct using connected components.
///
/// LCOM4 builds an undirected graph where methods are nodes. Two methods are
/// connected if they share access to a common field OR one calls the other.
/// LCOM4 = number of connected components in this graph.
///
/// - 0 = no methods
/// - 1 = cohesive (all methods are related)
/// - ≥2 = struct has unrelated groups of methods and could be split
fn compute_struct_lcom4(methods: &[&mdlr_core::Unit]) -> usize {
    if methods.is_empty() {
        return 0;
    }
    if methods.len() == 1 {
        return 1;
    }

    let n = methods.len();

    // Union-Find
    let mut parent: Vec<usize> = (0..n).collect();
    let mut rank: Vec<usize> = vec![0; n];

    fn find(parent: &mut Vec<usize>, i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }

    fn union(
        parent: &mut Vec<usize>,
        rank: &mut Vec<usize>,
        i: usize,
        j: usize,
    ) {
        let ri = find(parent, i);
        let rj = find(parent, j);
        if ri != rj {
            if rank[ri] < rank[rj] {
                parent[ri] = rj;
            } else if rank[ri] > rank[rj] {
                parent[rj] = ri;
            } else {
                parent[rj] = ri;
                rank[ri] += 1;
            }
        }
    }

    // Map each field to the method indices that access it
    let mut field_to_methods: HashMap<&String, Vec<usize>> = HashMap::new();
    for (idx, method) in methods.iter().enumerate() {
        for field in method.reads.iter().chain(method.writes.iter()) {
            field_to_methods.entry(field).or_default().push(idx);
        }
    }

    // Connect methods that share fields
    for method_indices in field_to_methods.values() {
        for window in method_indices.windows(2) {
            union(&mut parent, &mut rank, window[0], window[1]);
        }
    }

    // Connect methods that call each other (within this struct)
    let method_id_to_idx: HashMap<&str, usize> = methods
        .iter()
        .enumerate()
        .map(|(idx, m)| (m.id.as_str(), idx))
        .collect();

    for (idx, method) in methods.iter().enumerate() {
        for call in &method.calls {
            if let Some(&called_idx) = method_id_to_idx.get(call.as_str()) {
                union(&mut parent, &mut rank, idx, called_idx);
            }
        }
    }

    // Count connected components
    let mut roots: HashSet<usize> = HashSet::new();
    for i in 0..n {
        roots.insert(find(&mut parent, i));
    }

    roots.len()
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

    let lcom_values: HashMap<String, usize> = struct_methods
        .into_iter()
        .map(|(struct_id, methods)| {
            (struct_id, compute_struct_lcom4(&methods))
        })
        .collect();

    if lcom_values.is_empty() {
        return LcomMetrics { max: 0, mean: 0.0, distribution: vec![] };
    }

    let max = lcom_values.values().copied().max().unwrap_or(0);
    let sum: usize = lcom_values.values().sum();
    let mean = sum as f64 / lcom_values.len() as f64;

    let mut distribution: Vec<_> = lcom_values.into_iter().collect();
    distribution.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

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
            "LCOM4:          max={}, mean={:.1}",
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

        // Show least cohesive structs (LCOM4 >= 2 means should be split)
        let incohesive: Vec<_> = self
            .lcom
            .distribution
            .iter()
            .filter(|(_, lcom4)| *lcom4 >= 2)
            .take(10)
            .collect();

        if !incohesive.is_empty() {
            writeln!(f)?;
            writeln!(f, "Least Cohesive Structs (LCOM4 >= 2):")?;
            for (name, lcom4) in incohesive {
                writeln!(
                    f,
                    "  {} (LCOM4={}, {} connected components)",
                    name, lcom4, lcom4
                )?;
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
    fn test_lcom4_cohesive() {
        let mut graph = Graph::new();
        graph.add_unit(make_struct("Foo"));
        // Both methods access the same field - one connected component
        graph.add_unit(make_method("Foo::get_x", "Foo", vec!["x"], vec![]));
        graph.add_unit(make_method("Foo::set_x", "Foo", vec![], vec!["x"]));

        let metrics = StructMetrics::compute(&graph);

        // Methods share field "x", so LCOM4 = 1 (one connected component)
        assert_eq!(metrics.lcom.max, 1);
    }

    #[test]
    fn test_lcom4_incohesive() {
        let mut graph = Graph::new();
        graph.add_unit(make_struct("Foo"));
        // Methods access different fields - three disconnected components
        graph.add_unit(make_method("Foo::get_x", "Foo", vec!["x"], vec![]));
        graph.add_unit(make_method("Foo::get_y", "Foo", vec!["y"], vec![]));
        graph.add_unit(make_method("Foo::get_z", "Foo", vec!["z"], vec![]));

        let metrics = StructMetrics::compute(&graph);

        // No methods share fields, so LCOM4 = 3 (three connected components)
        assert_eq!(metrics.lcom.max, 3);
    }

    #[test]
    fn test_lcom4_connected_via_calls() {
        let mut graph = Graph::new();
        graph.add_unit(make_struct("Foo"));
        // get_x and get_y access different fields but validate calls get_x
        let mut validate =
            make_method("Foo::validate", "Foo", vec!["y"], vec![]);
        validate.calls = vec!["Foo::get_x".to_string()];
        graph.add_unit(make_method("Foo::get_x", "Foo", vec!["x"], vec![]));
        graph.add_unit(validate);

        let metrics = StructMetrics::compute(&graph);

        // validate calls get_x, so they're connected → LCOM4 = 1
        assert_eq!(metrics.lcom.max, 1);
    }

    #[test]
    fn test_lcom4_single_method() {
        let mut graph = Graph::new();
        graph.add_unit(make_struct("Foo"));
        graph.add_unit(make_method("Foo::run", "Foo", vec!["x"], vec![]));

        let metrics = StructMetrics::compute(&graph);

        // Single method → LCOM4 = 1
        assert_eq!(metrics.lcom.max, 1);
    }

    #[test]
    fn test_lcom4_mixed() {
        let mut graph = Graph::new();
        graph.add_unit(make_struct("Foo"));
        // Group 1: get_x and set_x share field "x"
        graph.add_unit(make_method("Foo::get_x", "Foo", vec!["x"], vec![]));
        graph.add_unit(make_method("Foo::set_x", "Foo", vec![], vec!["x"]));
        // Group 2: get_y is isolated
        graph.add_unit(make_method("Foo::get_y", "Foo", vec!["y"], vec![]));

        let metrics = StructMetrics::compute(&graph);

        // Two connected components: {get_x, set_x} and {get_y}
        assert_eq!(metrics.lcom.max, 2);
    }
}
