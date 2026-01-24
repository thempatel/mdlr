use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UnitKind {
    Function,
    Struct,
    Module,
    Trait,
    Impl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unit {
    pub id: String,
    pub kind: UnitKind,
    pub file: PathBuf,
    pub span: Span,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub calls: Vec<String>,
    pub tags: Vec<String>,
    /// Number of parameters (for functions)
    #[serde(default)]
    pub params: usize,
    /// Number of branch points (if/else/match arms/loops) for cyclomatic complexity
    #[serde(default)]
    pub branches: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EdgeKind {
    Calls,
    Reads,
    Writes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Graph {
    pub units: Vec<Unit>,
    pub edges: Vec<Edge>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_unit(&mut self, unit: Unit) {
        self.units.push(unit);
    }

    pub fn add_edge(&mut self, edge: Edge) {
        self.edges.push(edge);
    }

    pub fn merge(&mut self, other: Graph) {
        self.units.extend(other.units);
        self.edges.extend(other.edges);
    }

    pub fn clear(&mut self) {
        self.units.clear();
        self.edges.clear();
    }
}
