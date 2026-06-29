//! Disambiguate integration-test / example / bench crate ids.
//!
//! rust-analyzer compiles each `tests/*.rs` (and `examples/*.rs`,
//! `benches/*.rs`) file as its own crate named after the file *stem*, dropping
//! the owning cargo package. So every crate's `tests/extraction.rs` yields the
//! crate `extraction` and unit ids like `extraction::extract` collide across
//! packages — over-counting `duplication_pct`, conflating `fan_in`, and making
//! the symbol view return an arbitrary match (see ADR-0005).
//!
//! We restore global uniqueness by prefixing such a file's ids with
//! `<package>::<kind>` derived from the path, e.g.
//! `extraction::extract` -> `mdlr_extract_rust::tests::extraction::extract`.

use mdlr_core::Unit;
use std::path::Path;

/// Rewrite the `id`, `parent`, and self-crate `calls` of every unit extracted
/// from an integration-test/example/bench file so the stem-named crate is
/// qualified by its owning package. A no-op for ordinary `src/` files.
pub fn qualify_test_units(source_path: &str, units: &mut [Unit]) {
    let Some((prefix, stem)) = test_crate_prefix(source_path) else {
        return;
    };
    // A reference into this same stem-named crate is either the bare stem or
    // `<stem>::...`; anything else (e.g. a call into the lib crate) is left
    // alone.
    let self_ref = format!("{stem}::");
    let qualify = |s: &str| -> Option<String> {
        (s == stem || s.starts_with(&self_ref))
            .then(|| format!("{prefix}::{s}"))
    };

    for unit in units {
        if let Some(q) = qualify(&unit.id) {
            unit.id = q;
        }
        if let Some(parent) = unit.parent.as_deref().and_then(qualify) {
            unit.parent = Some(parent);
        }
        for call in unit.calls.iter_mut() {
            if let Some(q) = qualify(call) {
                *call = q;
            }
        }
    }
}

/// For a cargo target file `<pkg-dir>/{tests,benches,examples}/<stem>.rs`,
/// return (`<normalized-pkg>::<kind>`, `<stem>`). `None` for any other path.
///
/// The package name is taken from the directory above `tests/` (cargo requires
/// that layout); dashes are normalized to underscores to match rust-analyzer's
/// crate display names (`mdlr-extract-rust` -> `mdlr_extract_rust`).
fn test_crate_prefix(source_path: &str) -> Option<(String, String)> {
    let stem = Path::new(source_path).file_stem()?.to_str()?.to_string();
    let comps: Vec<&str> = source_path.split('/').collect();
    let n = comps.len();
    if n < 3 {
        return None;
    }
    let kind = comps[n - 2];
    if !matches!(kind, "tests" | "benches" | "examples") {
        return None;
    }
    let pkg = comps[n - 3].replace('-', "_");
    Some((format!("{pkg}::{kind}"), stem))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdlr_core::{Span, UnitKind};
    use std::path::PathBuf;

    fn unit(id: &str, parent: Option<&str>, calls: Vec<&str>) -> Unit {
        Unit {
            id: id.to_string(),
            kind: UnitKind::Function,
            file: PathBuf::from("x"),
            span: Span {
                start_line: 1,
                start_col: 0,
                end_line: 2,
                end_col: 0,
            },
            reads: vec![],
            writes: vec![],
            calls: calls.into_iter().map(String::from).collect(),
            tags: vec![],
            params: 0,
            branches: 0,
            max_scope_lines: 0,
            parent: parent.map(String::from),
            cognitive_complexity: 0,
            partial: false,
        }
    }

    #[test]
    fn qualifies_integration_test_ids_calls_and_parents() {
        let path = "crates/mdlr-extract-rust/tests/extraction.rs";
        let mut units = vec![
            unit("extraction::extract", None, vec![]),
            unit(
                "extraction::Foo::bar",
                Some("extraction::Foo"),
                // self-crate call qualified; lib-crate call left alone
                vec!["extraction::extract", "mdlr_extract_rust::extract"],
            ),
        ];

        qualify_test_units(path, &mut units);

        assert_eq!(
            units[0].id,
            "mdlr_extract_rust::tests::extraction::extract"
        );
        assert_eq!(
            units[1].id,
            "mdlr_extract_rust::tests::extraction::Foo::bar"
        );
        assert_eq!(
            units[1].parent.as_deref(),
            Some("mdlr_extract_rust::tests::extraction::Foo")
        );
        assert_eq!(
            units[1].calls,
            vec![
                "mdlr_extract_rust::tests::extraction::extract".to_string(),
                // unchanged: targets the lib crate, not this test crate
                "mdlr_extract_rust::extract".to_string(),
            ]
        );
    }

    #[test]
    fn two_packages_same_stem_no_longer_collide() {
        let mut a = vec![unit("extraction::extract", None, vec![])];
        let mut b = vec![unit("extraction::extract", None, vec![])];
        qualify_test_units(
            "crates/mdlr-extract-rust/tests/extraction.rs",
            &mut a,
        );
        qualify_test_units(
            "crates/mdlr-extract-ts/tests/extraction.rs",
            &mut b,
        );
        assert_ne!(a[0].id, b[0].id);
        assert_eq!(a[0].id, "mdlr_extract_rust::tests::extraction::extract");
        assert_eq!(b[0].id, "mdlr_extract_ts::tests::extraction::extract");
    }

    #[test]
    fn leaves_ordinary_src_files_untouched() {
        let path = "crates/mdlr-extract-rust/src/visitor.rs";
        let mut units =
            vec![unit("mdlr_extract_rust::visitor::make_span", None, vec![])];
        qualify_test_units(path, &mut units);
        assert_eq!(units[0].id, "mdlr_extract_rust::visitor::make_span");
    }
}
