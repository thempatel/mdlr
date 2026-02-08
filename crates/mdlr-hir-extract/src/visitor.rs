use mdlr_core::{Span, Unit, UnitKind};
use rustc_hir as hir;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::branches;
use crate::calls;
use crate::field_access;

/// Extract units from HIR for all requested source files.
///
/// Returns a map from source file path (as given in the mapping) to the units found in that file.
pub fn extract_units(
    tcx: TyCtxt<'_>,
    mapping: &HashMap<String, String>,
) -> HashMap<String, Vec<Unit>> {
    let mut results: HashMap<String, Vec<Unit>> = HashMap::new();

    // Pre-compute absolute paths for matching
    let cwd = std::env::current_dir().unwrap_or_default();
    let abs_mapping: HashMap<PathBuf, String> = mapping
        .keys()
        .map(|source| {
            let p = PathBuf::from(source);
            let abs = if p.is_absolute() {
                p
            } else {
                cwd.join(&p)
            };
            (abs, source.clone())
        })
        .collect();

    // Iterate all top-level items in the crate
    for item_id in tcx.hir_free_items() {
        let item = tcx.hir_item(item_id);
        let def_id = item.owner_id.def_id;
        let span = item.span;

        // Determine source file for this item
        let filename = tcx.sess.source_map().span_to_filename(span);
        let file_path = match &filename {
            FileName::Real(real) => real.local_path().map(|p| p.to_path_buf()),
            _ => None,
        };

        let file_path = match file_path {
            Some(p) => p,
            None => continue,
        };

        // Canonicalize to match against our mapping
        let abs_file = if file_path.is_absolute() {
            file_path.clone()
        } else {
            cwd.join(&file_path)
        };

        let source_key = match abs_mapping.get(&abs_file) {
            Some(key) => key.clone(),
            None => {
                // Try canonicalizing both paths
                let canonical = abs_file.canonicalize().ok();
                let found = abs_mapping.iter().find(|(k, _)| {
                    k.canonicalize().ok() == canonical
                });
                match found {
                    Some((_, key)) => key.clone(),
                    None => continue,
                }
            }
        };

        let units = results.entry(source_key.clone()).or_default();

        match &item.kind {
            hir::ItemKind::Struct(_ident, _generics, _variant_data) => {
                let id = tcx.def_path_str(def_id);
                let lo = tcx.sess.source_map().lookup_char_pos(span.lo());
                let hi = tcx.sess.source_map().lookup_char_pos(span.hi());

                units.push(Unit {
                    id,
                    kind: UnitKind::Struct,
                    file: PathBuf::from(&source_key),
                    span: make_span(&lo, &hi),
                    reads: vec![],
                    writes: vec![],
                    calls: vec![],
                    tags: vec![],
                    params: 0,
                    branches: 0,
                    parent: None,
                });
            }
            hir::ItemKind::Fn { sig, body: body_id, .. } => {
                let id = tcx.def_path_str(def_id);
                let lo = tcx.sess.source_map().lookup_char_pos(span.lo());
                let hi = tcx.sess.source_map().lookup_char_pos(span.hi());

                let body = tcx.hir_body(*body_id);
                let params = count_params(sig.decl);
                let branch_count = branches::count_branches(tcx, body);
                let call_targets = calls::extract_calls(tcx, def_id.to_def_id(), body);
                let (reads, writes) = field_access::extract_field_access(tcx, body);

                units.push(Unit {
                    id,
                    kind: UnitKind::Function,
                    file: PathBuf::from(&source_key),
                    span: make_span(&lo, &hi),
                    reads,
                    writes,
                    calls: call_targets,
                    tags: vec![],
                    params,
                    branches: branch_count,
                    parent: None,
                });
            }
            hir::ItemKind::Impl(impl_block) => {
                visit_impl_block(tcx, impl_block, &source_key, units);
            }
            _ => {}
        }
    }

    results
}

/// Visit an impl block and extract method units.
fn visit_impl_block(
    tcx: TyCtxt<'_>,
    impl_block: &hir::Impl<'_>,
    source_key: &str,
    units: &mut Vec<Unit>,
) {
    // Resolve the self type to find the parent struct
    let parent_id = resolve_impl_self_type(tcx, impl_block);

    for &impl_item_id in impl_block.items {
        let impl_item = tcx.hir_impl_item(impl_item_id);
        let def_id = impl_item.owner_id.def_id;
        let span = impl_item.span;

        match &impl_item.kind {
            hir::ImplItemKind::Fn(sig, body_id) => {
                let id = tcx.def_path_str(def_id);
                let lo = tcx.sess.source_map().lookup_char_pos(span.lo());
                let hi = tcx.sess.source_map().lookup_char_pos(span.hi());

                let body = tcx.hir_body(*body_id);
                let params = count_params_method(sig.decl);
                let branch_count = branches::count_branches(tcx, body);
                let call_targets = calls::extract_calls(tcx, def_id.to_def_id(), body);
                let (reads, writes) = field_access::extract_field_access(tcx, body);

                units.push(Unit {
                    id,
                    kind: UnitKind::Method,
                    file: PathBuf::from(source_key),
                    span: make_span(&lo, &hi),
                    reads,
                    writes,
                    calls: call_targets,
                    tags: vec![],
                    params,
                    branches: branch_count,
                    parent: parent_id.clone(),
                });
            }
            _ => {}
        }
    }
}

/// Resolve the self type of an impl block to a struct's def_path_str.
fn resolve_impl_self_type(tcx: TyCtxt<'_>, impl_block: &hir::Impl<'_>) -> Option<String> {
    if let hir::TyKind::Path(hir::QPath::Resolved(_, path)) = &impl_block.self_ty.kind {
        if let hir::def::Res::Def(_, def_id) = path.res {
            return Some(tcx.def_path_str(def_id));
        }
    }
    None
}

/// Count parameters for a standalone function (all params count).
fn count_params(decl: &hir::FnDecl<'_>) -> usize {
    decl.inputs.len()
}

/// Count parameters for a method, excluding self/&self/&mut self.
fn count_params_method(decl: &hir::FnDecl<'_>) -> usize {
    if decl.implicit_self.has_implicit_self() {
        decl.inputs.len().saturating_sub(1)
    } else {
        decl.inputs.len()
    }
}

fn make_span(
    lo: &rustc_span::Loc,
    hi: &rustc_span::Loc,
) -> Span {
    Span {
        start_line: lo.line,
        start_col: lo.col.0,
        end_line: hi.line,
        end_col: hi.col.0,
    }
}
