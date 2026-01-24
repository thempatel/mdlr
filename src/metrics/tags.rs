use crate::cache::SemanticTags;
use crate::graph::Graph;
use std::collections::{HashMap, HashSet};

/// Metrics computed from semantic tags
#[derive(Debug, Clone)]
pub struct TagMetrics {
    /// Total number of units
    pub total_units: usize,
    /// Number of units with at least one semantic tag
    pub tagged_units: usize,
    /// Tag coverage as a percentage (0.0 to 1.0)
    pub tag_coverage: f64,
    /// Distribution by namespace (namespace -> count of units)
    pub namespace_distribution: HashMap<String, usize>,
    /// Per-namespace breakdown (namespace -> (value -> count))
    pub namespace_values: HashMap<String, HashMap<String, usize>>,
    /// Conceptual metrics (only computed when tags exist)
    pub conceptual: Option<ConceptualMetrics>,
}

/// Metrics that analyze conceptual relationships via tags
#[derive(Debug, Clone)]
pub struct ConceptualMetrics {
    /// How many concepts each unit touches (unit_id -> count of distinct tags)
    pub conceptual_fan_out: FanDistribution,
    /// How scattered each concept is across files
    pub concept_scattering: Vec<ConceptScatter>,
    /// Edges that cross between different tag values within a namespace
    pub cross_concept_edges: CrossConceptEdges,
}

/// Distribution of fan values
#[derive(Debug, Clone)]
pub struct FanDistribution {
    pub max: usize,
    pub mean: f64,
    /// Top units by count: (unit_id, count)
    pub top: Vec<(String, usize)>,
}

/// Scattering metrics for a single concept
#[derive(Debug, Clone)]
pub struct ConceptScatter {
    /// The tag (e.g., "domain:auth")
    pub tag: String,
    /// Number of units with this tag
    pub unit_count: usize,
    /// Number of distinct files containing units with this tag
    pub file_count: usize,
    /// Scattering ratio: file_count / unit_count (1.0 = perfectly scattered, low = cohesive)
    pub scatter_ratio: f64,
}

/// Cross-concept edge analysis
#[derive(Debug, Clone)]
pub struct CrossConceptEdges {
    /// Total edges between tagged units
    pub total_tagged_edges: usize,
    /// Edges that cross concept boundaries (within same namespace)
    pub cross_concept_count: usize,
    /// Ratio of cross-concept edges
    pub cross_concept_ratio: f64,
    /// Breakdown by namespace: namespace -> (from_value, to_value, count)
    pub by_namespace: HashMap<String, Vec<(String, String, usize)>>,
}

impl TagMetrics {
    /// Compute tag metrics from graph and semantic tags
    pub fn compute(graph: &Graph, tags: &SemanticTags) -> Self {
        let total_units = graph.units.len();

        // Count units that have tags
        let tagged_units = graph
            .units
            .iter()
            .filter(|u| !tags.get_tags(&u.id).is_empty())
            .count();

        let tag_coverage = if total_units > 0 {
            tagged_units as f64 / total_units as f64
        } else {
            0.0
        };

        // Build namespace distributions
        let mut namespace_distribution: HashMap<String, usize> = HashMap::new();
        let mut namespace_values: HashMap<String, HashMap<String, usize>> = HashMap::new();

        for unit in &graph.units {
            for tag in tags.get_tags(&unit.id) {
                if let Some((namespace, value)) = tag.split_once(':') {
                    *namespace_distribution.entry(namespace.to_string()).or_insert(0) += 1;

                    let values = namespace_values
                        .entry(namespace.to_string())
                        .or_default();
                    *values.entry(value.to_string()).or_insert(0) += 1;
                }
            }
        }

        // Compute conceptual metrics only if we have tagged units
        let conceptual = if tagged_units > 0 {
            Some(ConceptualMetrics::compute(graph, tags))
        } else {
            None
        };

        Self {
            total_units,
            tagged_units,
            tag_coverage,
            namespace_distribution,
            namespace_values,
            conceptual,
        }
    }

    /// Check if there are any tags
    pub fn has_tags(&self) -> bool {
        self.tagged_units > 0
    }
}

impl ConceptualMetrics {
    fn compute(graph: &Graph, tags: &SemanticTags) -> Self {
        // 1. Conceptual Fan-Out: count distinct tags per unit
        let mut fan_out_counts: HashMap<String, usize> = HashMap::new();
        for unit in &graph.units {
            let unit_tags = tags.get_tags(&unit.id);
            if !unit_tags.is_empty() {
                fan_out_counts.insert(unit.id.clone(), unit_tags.len());
            }
        }
        let conceptual_fan_out = FanDistribution::from_counts(fan_out_counts);

        // 2. Concept Scattering: for each tag, count units and files
        let mut tag_units: HashMap<String, HashSet<String>> = HashMap::new();
        let mut tag_files: HashMap<String, HashSet<String>> = HashMap::new();

        for unit in &graph.units {
            for tag in tags.get_tags(&unit.id) {
                tag_units
                    .entry(tag.clone())
                    .or_default()
                    .insert(unit.id.clone());
                tag_files
                    .entry(tag.clone())
                    .or_default()
                    .insert(unit.file.to_string_lossy().to_string());
            }
        }

        let mut concept_scattering: Vec<ConceptScatter> = tag_units
            .iter()
            .map(|(tag, units)| {
                let unit_count = units.len();
                let file_count = tag_files.get(tag).map(|f| f.len()).unwrap_or(0);
                let scatter_ratio = if unit_count > 0 {
                    file_count as f64 / unit_count as f64
                } else {
                    0.0
                };
                ConceptScatter {
                    tag: tag.clone(),
                    unit_count,
                    file_count,
                    scatter_ratio,
                }
            })
            .collect();

        // Sort by scatter_ratio descending (most scattered first)
        concept_scattering.sort_by(|a, b| {
            b.scatter_ratio
                .partial_cmp(&a.scatter_ratio)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.unit_count.cmp(&a.unit_count))
        });

        // 3. Cross-Concept Edges
        let cross_concept_edges = compute_cross_concept_edges(graph, tags);

        Self {
            conceptual_fan_out,
            concept_scattering,
            cross_concept_edges,
        }
    }
}

impl FanDistribution {
    fn from_counts(counts: HashMap<String, usize>) -> Self {
        if counts.is_empty() {
            return Self {
                max: 0,
                mean: 0.0,
                top: vec![],
            };
        }

        let max = counts.values().copied().max().unwrap_or(0);
        let sum: usize = counts.values().sum();
        let mean = sum as f64 / counts.len() as f64;

        let mut top: Vec<_> = counts.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top.truncate(10);

        Self { max, mean, top }
    }
}

fn compute_cross_concept_edges(graph: &Graph, tags: &SemanticTags) -> CrossConceptEdges {
    // Build unit_id -> tags lookup
    let unit_tags: HashMap<String, Vec<String>> = graph
        .units
        .iter()
        .map(|u| (u.id.clone(), tags.get_tags(&u.id).to_vec()))
        .collect();

    let mut total_tagged_edges = 0;
    let mut cross_concept_count = 0;
    let mut by_namespace: HashMap<String, HashMap<(String, String), usize>> = HashMap::new();

    for edge in &graph.edges {
        let from_tags = unit_tags.get(&edge.from).map(|t| t.as_slice()).unwrap_or(&[]);
        let to_tags = unit_tags.get(&edge.to).map(|t| t.as_slice()).unwrap_or(&[]);

        // Skip edges where neither unit is tagged
        if from_tags.is_empty() && to_tags.is_empty() {
            continue;
        }

        total_tagged_edges += 1;

        // Check for cross-concept edges within each namespace
        for from_tag in from_tags {
            if let Some((from_ns, from_val)) = from_tag.split_once(':') {
                for to_tag in to_tags {
                    if let Some((to_ns, to_val)) = to_tag.split_once(':') {
                        // Same namespace, different value = cross-concept
                        if from_ns == to_ns && from_val != to_val {
                            cross_concept_count += 1;

                            let ns_map = by_namespace.entry(from_ns.to_string()).or_default();
                            let key = if from_val < to_val {
                                (from_val.to_string(), to_val.to_string())
                            } else {
                                (to_val.to_string(), from_val.to_string())
                            };
                            *ns_map.entry(key).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }

    let cross_concept_ratio = if total_tagged_edges > 0 {
        cross_concept_count as f64 / total_tagged_edges as f64
    } else {
        0.0
    };

    // Convert by_namespace to sorted vec format
    let by_namespace: HashMap<String, Vec<(String, String, usize)>> = by_namespace
        .into_iter()
        .map(|(ns, pairs)| {
            let mut pairs_vec: Vec<_> = pairs
                .into_iter()
                .map(|((a, b), count)| (a, b, count))
                .collect();
            pairs_vec.sort_by(|a, b| b.2.cmp(&a.2));
            (ns, pairs_vec)
        })
        .collect();

    CrossConceptEdges {
        total_tagged_edges,
        cross_concept_count,
        cross_concept_ratio,
        by_namespace,
    }
}

impl std::fmt::Display for TagMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Semantic Tags")?;
        writeln!(f, "=============")?;
        writeln!(f)?;
        writeln!(
            f,
            "Coverage: {:.1}% ({}/{} units tagged)",
            self.tag_coverage * 100.0,
            self.tagged_units,
            self.total_units
        )?;

        if !self.namespace_distribution.is_empty() {
            writeln!(f)?;
            writeln!(f, "By Namespace:")?;

            let mut namespaces: Vec<_> = self.namespace_distribution.iter().collect();
            namespaces.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));

            for (namespace, count) in namespaces {
                writeln!(f, "  {}: {} units", namespace, count)?;

                if let Some(values) = self.namespace_values.get(namespace) {
                    let mut values_vec: Vec<_> = values.iter().collect();
                    values_vec.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));

                    for (value, vcount) in values_vec.iter().take(5) {
                        writeln!(f, "    {}:{} ({})", namespace, value, vcount)?;
                    }
                }
            }
        }

        if let Some(ref conceptual) = self.conceptual {
            writeln!(f)?;
            write!(f, "{}", conceptual)?;
        }

        Ok(())
    }
}

impl std::fmt::Display for ConceptualMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Conceptual Fan-Out
        if self.conceptual_fan_out.max > 1 {
            writeln!(f, "Conceptual Fan-Out (tags per unit):")?;
            writeln!(
                f,
                "  max={}, mean={:.2}",
                self.conceptual_fan_out.max, self.conceptual_fan_out.mean
            )?;

            let overloaded: Vec<_> = self
                .conceptual_fan_out
                .top
                .iter()
                .filter(|(_, count)| *count > 1)
                .take(5)
                .collect();

            if !overloaded.is_empty() {
                writeln!(f)?;
                writeln!(f, "  Potential conceptual overload:")?;
                for (unit, count) in overloaded {
                    writeln!(f, "    {} ({} concepts)", unit, count)?;
                }
            }
            writeln!(f)?;
        }

        // Concept Scattering
        let scattered: Vec<_> = self
            .concept_scattering
            .iter()
            .filter(|s| s.file_count > 1 && s.scatter_ratio > 0.5)
            .take(5)
            .collect();

        if !scattered.is_empty() {
            writeln!(f, "Concept Scattering (high = spread across files):")?;
            for scatter in scattered {
                writeln!(
                    f,
                    "  {} - {} units across {} files (ratio: {:.2})",
                    scatter.tag, scatter.unit_count, scatter.file_count, scatter.scatter_ratio
                )?;
            }
            writeln!(f)?;
        }

        // Cross-Concept Edges
        if self.cross_concept_edges.cross_concept_count > 0 {
            writeln!(f, "Cross-Concept Coupling:")?;
            writeln!(
                f,
                "  {}/{} edges cross concept boundaries ({:.1}%)",
                self.cross_concept_edges.cross_concept_count,
                self.cross_concept_edges.total_tagged_edges,
                self.cross_concept_edges.cross_concept_ratio * 100.0
            )?;

            for (namespace, pairs) in &self.cross_concept_edges.by_namespace {
                if !pairs.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "  {}:", namespace)?;
                    for (from, to, count) in pairs.iter().take(5) {
                        writeln!(f, "    {} <-> {} ({} edges)", from, to, count)?;
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, EdgeKind, Span, Unit, UnitKind};
    use std::path::PathBuf;

    fn make_unit(id: &str, file: &str) -> Unit {
        Unit {
            id: id.to_string(),
            kind: UnitKind::Function,
            file: PathBuf::from(file),
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
            impl_trait: None,
            impl_type: None,
        }
    }

    #[test]
    fn test_empty_tags() {
        let mut graph = Graph::new();
        graph.add_unit(make_unit("a", "test.rs"));
        graph.add_unit(make_unit("b", "test.rs"));

        let tags = SemanticTags::new();
        let metrics = TagMetrics::compute(&graph, &tags);

        assert_eq!(metrics.total_units, 2);
        assert_eq!(metrics.tagged_units, 0);
        assert_eq!(metrics.tag_coverage, 0.0);
        assert!(!metrics.has_tags());
        assert!(metrics.conceptual.is_none());
    }

    #[test]
    fn test_partial_coverage() {
        let mut graph = Graph::new();
        graph.add_unit(make_unit("a", "test.rs"));
        graph.add_unit(make_unit("b", "test.rs"));
        graph.add_unit(make_unit("c", "test.rs"));
        graph.add_unit(make_unit("d", "test.rs"));

        let mut tags = SemanticTags::new();
        tags.add_tag("a", "domain:auth").unwrap();
        tags.add_tag("b", "domain:billing").unwrap();

        let metrics = TagMetrics::compute(&graph, &tags);

        assert_eq!(metrics.total_units, 4);
        assert_eq!(metrics.tagged_units, 2);
        assert_eq!(metrics.tag_coverage, 0.5);
        assert!(metrics.has_tags());
        assert_eq!(*metrics.namespace_distribution.get("domain").unwrap(), 2);
    }

    #[test]
    fn test_conceptual_fan_out() {
        let mut graph = Graph::new();
        graph.add_unit(make_unit("a", "test.rs"));
        graph.add_unit(make_unit("b", "test.rs"));

        let mut tags = SemanticTags::new();
        // 'a' has 3 concepts (high fan-out)
        tags.add_tag("a", "domain:auth").unwrap();
        tags.add_tag("a", "domain:billing").unwrap();
        tags.add_tag("a", "layer:api").unwrap();
        // 'b' has 1 concept
        tags.add_tag("b", "domain:auth").unwrap();

        let metrics = TagMetrics::compute(&graph, &tags);
        let conceptual = metrics.conceptual.unwrap();

        assert_eq!(conceptual.conceptual_fan_out.max, 3);
        assert_eq!(conceptual.conceptual_fan_out.mean, 2.0); // (3 + 1) / 2
    }

    #[test]
    fn test_concept_scattering() {
        let mut graph = Graph::new();
        graph.add_unit(make_unit("a", "auth.rs"));
        graph.add_unit(make_unit("b", "billing.rs"));
        graph.add_unit(make_unit("c", "utils.rs"));

        let mut tags = SemanticTags::new();
        // domain:auth is scattered across 3 files (bad cohesion)
        tags.add_tag("a", "domain:auth").unwrap();
        tags.add_tag("b", "domain:auth").unwrap();
        tags.add_tag("c", "domain:auth").unwrap();

        let metrics = TagMetrics::compute(&graph, &tags);
        let conceptual = metrics.conceptual.unwrap();

        let auth_scatter = conceptual
            .concept_scattering
            .iter()
            .find(|s| s.tag == "domain:auth")
            .unwrap();

        assert_eq!(auth_scatter.unit_count, 3);
        assert_eq!(auth_scatter.file_count, 3);
        assert_eq!(auth_scatter.scatter_ratio, 1.0); // Perfectly scattered
    }

    #[test]
    fn test_cross_concept_edges() {
        let mut graph = Graph::new();
        graph.add_unit(make_unit("auth_fn", "auth.rs"));
        graph.add_unit(make_unit("billing_fn", "billing.rs"));
        graph.add_unit(make_unit("auth_helper", "auth.rs"));

        // auth_fn calls billing_fn (cross-domain)
        graph.add_edge(Edge {
            from: "auth_fn".to_string(),
            to: "billing_fn".to_string(),
            kind: EdgeKind::Calls,
        });
        // auth_fn calls auth_helper (same domain)
        graph.add_edge(Edge {
            from: "auth_fn".to_string(),
            to: "auth_helper".to_string(),
            kind: EdgeKind::Calls,
        });

        let mut tags = SemanticTags::new();
        tags.add_tag("auth_fn", "domain:auth").unwrap();
        tags.add_tag("billing_fn", "domain:billing").unwrap();
        tags.add_tag("auth_helper", "domain:auth").unwrap();

        let metrics = TagMetrics::compute(&graph, &tags);
        let conceptual = metrics.conceptual.unwrap();

        assert_eq!(conceptual.cross_concept_edges.total_tagged_edges, 2);
        assert_eq!(conceptual.cross_concept_edges.cross_concept_count, 1);
        assert_eq!(conceptual.cross_concept_edges.cross_concept_ratio, 0.5);
    }

    #[test]
    fn test_namespace_values() {
        let mut graph = Graph::new();
        graph.add_unit(make_unit("a", "test.rs"));
        graph.add_unit(make_unit("b", "test.rs"));

        let mut tags = SemanticTags::new();
        tags.add_tag("a", "domain:auth").unwrap();
        tags.add_tag("a", "layer:api").unwrap();
        tags.add_tag("b", "domain:auth").unwrap();

        let metrics = TagMetrics::compute(&graph, &tags);

        let domain_values = metrics.namespace_values.get("domain").unwrap();
        assert_eq!(*domain_values.get("auth").unwrap(), 2);

        let layer_values = metrics.namespace_values.get("layer").unwrap();
        assert_eq!(*layer_values.get("api").unwrap(), 1);
    }
}
