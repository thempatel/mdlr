use crate::graph::{Graph, UnitKind};
use std::collections::{HashMap, HashSet};

/// Metrics for impl blocks (class-like constructs)
#[derive(Debug, Clone)]
pub struct ImplMetrics {
    /// Methods per impl block
    pub methods_per_impl: MethodsPerImpl,
    /// Trait implementations per type
    pub traits_per_type: TraitsPerType,
    /// LCOM (Lack of Cohesion of Methods) per impl
    pub lcom: LcomMetrics,
}

#[derive(Debug, Clone)]
pub struct MethodsPerImpl {
    pub max: usize,
    pub mean: f64,
    pub p90: usize,
    /// Impl blocks sorted by method count descending
    pub distribution: Vec<(String, usize)>,
}

#[derive(Debug, Clone)]
pub struct TraitsPerType {
    pub max: usize,
    pub mean: f64,
    /// Types sorted by trait count descending
    pub distribution: Vec<(String, usize)>,
}

#[derive(Debug, Clone)]
pub struct LcomMetrics {
    pub max: f64,
    pub mean: f64,
    /// Impl blocks sorted by LCOM descending (higher = less cohesive)
    pub distribution: Vec<(String, f64)>,
}

impl ImplMetrics {
    pub fn compute(graph: &Graph) -> Self {
        let methods_per_impl = compute_methods_per_impl(graph);
        let traits_per_type = compute_traits_per_type(graph);
        let lcom = compute_lcom(graph);

        Self {
            methods_per_impl,
            traits_per_type,
            lcom,
        }
    }

    pub fn has_impls(&self) -> bool {
        !self.methods_per_impl.distribution.is_empty()
    }
}

fn compute_methods_per_impl(graph: &Graph) -> MethodsPerImpl {
    let mut impl_method_count: HashMap<String, usize> = HashMap::new();

    // Initialize all impl blocks with 0 methods
    for unit in &graph.units {
        if unit.kind == UnitKind::Impl {
            impl_method_count.insert(unit.id.clone(), 0);
        }
    }

    // Count methods per impl
    for unit in &graph.units {
        if unit.kind == UnitKind::Function {
            if let Some(ref parent) = unit.parent {
                *impl_method_count.entry(parent.clone()).or_insert(0) += 1;
            }
        }
    }

    if impl_method_count.is_empty() {
        return MethodsPerImpl {
            max: 0,
            mean: 0.0,
            p90: 0,
            distribution: vec![],
        };
    }

    let max = impl_method_count.values().copied().max().unwrap_or(0);
    let sum: usize = impl_method_count.values().sum();
    let mean = sum as f64 / impl_method_count.len() as f64;

    let mut sorted_values: Vec<usize> = impl_method_count.values().copied().collect();
    sorted_values.sort();
    let p90_idx = (sorted_values.len() as f64 * 0.9).ceil() as usize - 1;
    let p90 = sorted_values.get(p90_idx).copied().unwrap_or(max);

    let mut distribution: Vec<_> = impl_method_count.into_iter().collect();
    distribution.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    MethodsPerImpl {
        max,
        mean,
        p90,
        distribution,
    }
}

fn compute_traits_per_type(graph: &Graph) -> TraitsPerType {
    let mut type_traits: HashMap<String, HashSet<String>> = HashMap::new();

    for unit in &graph.units {
        if unit.kind == UnitKind::Impl {
            if let (Some(impl_type), Some(impl_trait)) = (&unit.impl_type, &unit.impl_trait)
            {
                type_traits
                    .entry(impl_type.clone())
                    .or_default()
                    .insert(impl_trait.clone());
            }
        }
    }

    if type_traits.is_empty() {
        return TraitsPerType {
            max: 0,
            mean: 0.0,
            distribution: vec![],
        };
    }

    let counts: HashMap<String, usize> = type_traits
        .into_iter()
        .map(|(t, traits)| (t, traits.len()))
        .collect();

    let max = counts.values().copied().max().unwrap_or(0);
    let sum: usize = counts.values().sum();
    let mean = sum as f64 / counts.len() as f64;

    let mut distribution: Vec<_> = counts.into_iter().collect();
    distribution.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    TraitsPerType {
        max,
        mean,
        distribution,
    }
}

fn compute_lcom(graph: &Graph) -> LcomMetrics {
    // Group methods by impl
    let mut impl_methods: HashMap<String, Vec<&crate::graph::Unit>> = HashMap::new();

    for unit in &graph.units {
        if unit.kind == UnitKind::Function {
            if let Some(ref parent) = unit.parent {
                impl_methods.entry(parent.clone()).or_default().push(unit);
            }
        }
    }

    let mut lcom_values: HashMap<String, f64> = HashMap::new();

    for (impl_id, methods) in impl_methods {
        if methods.len() < 2 {
            // LCOM is 0 for single-method impls
            lcom_values.insert(impl_id, 0.0);
            continue;
        }

        // Compute LCOM using Henderson-Sellers formula:
        // LCOM = (m - sum(mA)/a) / (m - 1)
        // where m = number of methods, a = number of attributes
        // mA = number of methods accessing attribute A
        //
        // Simplified: count pairs of methods that share fields vs don't share

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
        let normalized_lcom = if total_pairs > 0 {
            lcom / total_pairs as f64
        } else {
            0.0
        };

        lcom_values.insert(impl_id, normalized_lcom);
    }

    if lcom_values.is_empty() {
        return LcomMetrics {
            max: 0.0,
            mean: 0.0,
            distribution: vec![],
        };
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

    LcomMetrics {
        max,
        mean,
        distribution,
    }
}

impl std::fmt::Display for ImplMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Impl Metrics")?;
        writeln!(f, "============")?;
        writeln!(f)?;

        writeln!(
            f,
            "Methods/Impl: max={}, mean={:.1}, p90={}",
            self.methods_per_impl.max, self.methods_per_impl.mean, self.methods_per_impl.p90
        )?;

        if self.traits_per_type.max > 0 {
            writeln!(
                f,
                "Traits/Type:  max={}, mean={:.1}",
                self.traits_per_type.max, self.traits_per_type.mean
            )?;
        }

        writeln!(
            f,
            "LCOM:         max={:.2}, mean={:.2}",
            self.lcom.max, self.lcom.mean
        )?;

        // Show largest impls
        let large: Vec<_> = self
            .methods_per_impl
            .distribution
            .iter()
            .filter(|(_, c)| *c > 5)
            .take(10)
            .collect();

        if !large.is_empty() {
            writeln!(f)?;
            writeln!(f, "Largest Impls:")?;
            for (name, count) in large {
                writeln!(f, "  {} ({} methods)", name, count)?;
            }
        }

        // Show types with many traits
        let many_traits: Vec<_> = self
            .traits_per_type
            .distribution
            .iter()
            .filter(|(_, c)| *c > 2)
            .take(10)
            .collect();

        if !many_traits.is_empty() {
            writeln!(f)?;
            writeln!(f, "Types with Many Traits:")?;
            for (name, count) in many_traits {
                writeln!(f, "  {} ({} traits)", name, count)?;
            }
        }

        // Show least cohesive impls
        let incohesive: Vec<_> = self
            .lcom
            .distribution
            .iter()
            .filter(|(_, lcom)| *lcom > 0.5)
            .take(10)
            .collect();

        if !incohesive.is_empty() {
            writeln!(f)?;
            writeln!(f, "Least Cohesive Impls (high LCOM):")?;
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
    use crate::graph::{Span, Unit};
    use std::path::PathBuf;

    fn make_impl(id: &str, impl_type: &str, impl_trait: Option<&str>) -> Unit {
        Unit {
            id: id.to_string(),
            kind: UnitKind::Impl,
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
            impl_trait: impl_trait.map(|s| s.to_string()),
            impl_type: Some(impl_type.to_string()),
        }
    }

    fn make_method(id: &str, parent: &str, reads: Vec<&str>, writes: Vec<&str>) -> Unit {
        Unit {
            id: id.to_string(),
            kind: UnitKind::Function,
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
            impl_trait: None,
            impl_type: None,
        }
    }

    #[test]
    fn test_methods_per_impl() {
        let mut graph = Graph::new();
        graph.add_unit(make_impl("impl Foo", "Foo", None));
        graph.add_unit(make_impl("impl Bar", "Bar", None));
        graph.add_unit(make_method("new", "impl Foo", vec![], vec![]));
        graph.add_unit(make_method("get", "impl Foo", vec![], vec![]));
        graph.add_unit(make_method("set", "impl Foo", vec![], vec![]));
        graph.add_unit(make_method("run", "impl Bar", vec![], vec![]));

        let metrics = ImplMetrics::compute(&graph);

        assert_eq!(metrics.methods_per_impl.max, 3);
        assert_eq!(metrics.methods_per_impl.distribution[0].0, "impl Foo");
        assert_eq!(metrics.methods_per_impl.distribution[0].1, 3);
    }

    #[test]
    fn test_traits_per_type() {
        let mut graph = Graph::new();
        graph.add_unit(make_impl("impl Display for Foo", "Foo", Some("Display")));
        graph.add_unit(make_impl("impl Debug for Foo", "Foo", Some("Debug")));
        graph.add_unit(make_impl("impl Clone for Foo", "Foo", Some("Clone")));
        graph.add_unit(make_impl("impl Display for Bar", "Bar", Some("Display")));

        let metrics = ImplMetrics::compute(&graph);

        assert_eq!(metrics.traits_per_type.max, 3);
        assert_eq!(metrics.traits_per_type.distribution[0].0, "Foo");
        assert_eq!(metrics.traits_per_type.distribution[0].1, 3);
    }

    #[test]
    fn test_lcom_cohesive() {
        let mut graph = Graph::new();
        graph.add_unit(make_impl("impl Foo", "Foo", None));
        // Both methods access the same field - cohesive
        graph.add_unit(make_method("get_x", "impl Foo", vec!["x"], vec![]));
        graph.add_unit(make_method("set_x", "impl Foo", vec![], vec!["x"]));

        let metrics = ImplMetrics::compute(&graph);

        // Methods share field access, so LCOM should be 0
        assert_eq!(metrics.lcom.max, 0.0);
    }

    #[test]
    fn test_lcom_incohesive() {
        let mut graph = Graph::new();
        graph.add_unit(make_impl("impl Foo", "Foo", None));
        // Methods access different fields - incohesive
        graph.add_unit(make_method("get_x", "impl Foo", vec!["x"], vec![]));
        graph.add_unit(make_method("get_y", "impl Foo", vec!["y"], vec![]));
        graph.add_unit(make_method("get_z", "impl Foo", vec!["z"], vec![]));

        let metrics = ImplMetrics::compute(&graph);

        // No methods share field access, so LCOM should be > 0
        assert!(metrics.lcom.max > 0.0);
    }
}
