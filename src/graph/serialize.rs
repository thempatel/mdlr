use super::types::Graph;
use anyhow::Result;

pub fn to_json(graph: &Graph) -> Result<String> {
    Ok(serde_json::to_string_pretty(graph)?)
}

pub fn from_json(json: &str) -> Result<Graph> {
    Ok(serde_json::from_str(json)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Edge, EdgeKind, Span, Unit, UnitKind};
    use std::path::PathBuf;

    #[test]
    fn test_roundtrip() {
        let mut graph = Graph::new();
        graph.add_unit(Unit {
            id: "test::foo".to_string(),
            kind: UnitKind::Function,
            file: PathBuf::from("test.rs"),
            span: Span {
                start_line: 1,
                start_col: 0,
                end_line: 5,
                end_col: 1,
            },
            reads: vec![],
            writes: vec![],
            calls: vec!["test::bar".to_string()],
            tags: vec![],
            params: 0,
            branches: 0,
        });
        graph.add_edge(Edge {
            from: "test::foo".to_string(),
            to: "test::bar".to_string(),
            kind: EdgeKind::Calls,
        });

        let json = to_json(&graph).unwrap();
        let restored = from_json(&json).unwrap();

        assert_eq!(graph.units.len(), restored.units.len());
        assert_eq!(graph.edges.len(), restored.edges.len());
    }
}
