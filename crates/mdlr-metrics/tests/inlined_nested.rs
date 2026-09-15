//! `inlined_size` over units the TS extractor actually produces.
//!
//! The unit fixtures in `inlined.rs` choose their own spans, so they cannot
//! show what nesting looks like coming out of an extractor: a nested function
//! becomes its own unit whose span sits inside its parent's, and the parent's
//! own size already counts those lines.

use mdlr_core::FileCacheEntry;
use std::path::{Path, PathBuf};

fn graph_of(source: &str) -> mdlr_core::Graph {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let root = tmp.path();

    let file_path = root.join("src/nested.ts");
    std::fs::create_dir_all(file_path.parent().unwrap()).expect("mkdir");
    std::fs::write(&file_path, source).expect("write source");

    let output_dir = root.join("output");
    std::fs::create_dir_all(&output_dir).expect("mkdir output");
    mdlr_extract_ts::extract(root, &output_dir, Some(1))
        .expect("run extractor");

    let mut units = Vec::new();
    for json_file in json_files(&output_dir) {
        let content = std::fs::read_to_string(&json_file)
            .unwrap_or_else(|e| panic!("read {}: {e}", json_file.display()));
        let entry: FileCacheEntry = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("parse {}: {e}", json_file.display()));
        units.extend(entry.units);
    }
    assert!(!units.is_empty(), "extractor produced no units");
    mdlr_core::build(units)
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(json_files(&path));
            } else if path.extension().is_some_and(|e| e == "json") {
                results.push(path);
            }
        }
    }
    results
}

fn size_of(m: &mdlr_metrics::InlinedMetrics, id: &str) -> usize {
    m.inlined_size
        .distribution
        .iter()
        .find(|(k, _)| k == id)
        .map(|(_, v)| *v)
        .unwrap_or_else(|| {
            let ids: Vec<&str> = m
                .inlined_size
                .distribution
                .iter()
                .map(|(k, _)| k.as_str())
                .collect();
            panic!("{id} not in distribution; have {ids:?}")
        })
}

/// `outer` owns four nested helpers, but their lines are inside its own span.
/// Absorbing them would report a unit larger than the file it lives in, and
/// the width would clear the gate that decides whether the row is listed.
#[test]
fn nested_helpers_are_not_double_counted() {
    let source = r#"
export function outer(n: number): number {
    function stepOne(v: number): number {
        return v + 1;
    }
    function stepTwo(v: number): number {
        return v + 2;
    }
    function stepThree(v: number): number {
        return v + 3;
    }
    function stepFour(v: number): number {
        return v + 4;
    }
    return stepOne(stepTwo(stepThree(stepFour(n))));
}
"#;

    let graph = graph_of(source);
    let outer = graph
        .units
        .iter()
        .find(|u| u.id.ends_with("::outer"))
        .expect("outer unit");
    let own = outer.span.end_line - outer.span.start_line + 1;

    let m = mdlr_metrics::compute_inlined(&graph);
    assert_eq!(
        size_of(&m, &outer.id),
        own,
        "nested helpers must not be absorbed into their enclosing unit",
    );
    assert_eq!(
        m.exclusive_fanout[&outer.id], 0,
        "nested helpers must not count toward the gate width",
    );
}

/// A sibling helper at file scope is still absorbed — the containment check
/// must not disable the metric.
#[test]
fn sibling_helper_is_still_absorbed() {
    let source = r#"
function helper(v: number): number {
    return v + 1;
}

export function parent(n: number): number {
    return helper(n);
}
"#;

    let graph = graph_of(source);
    let parent = graph
        .units
        .iter()
        .find(|u| u.id.ends_with("::parent"))
        .expect("parent unit");
    let helper = graph
        .units
        .iter()
        .find(|u| u.id.ends_with("::helper"))
        .expect("helper unit");
    let parent_own = parent.span.end_line - parent.span.start_line + 1;
    let helper_own = helper.span.end_line - helper.span.start_line + 1;

    let m = mdlr_metrics::compute_inlined(&graph);
    assert_eq!(size_of(&m, &parent.id), parent_own + helper_own);
    assert_eq!(m.exclusive_fanout[&parent.id], 1);
}
