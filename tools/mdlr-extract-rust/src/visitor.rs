use mdlr_core::{Span, Unit, UnitKind};
use rustc_hir as hir;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;
use rustc_span::def_id::{DefId, LOCAL_CRATE};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::branches;
use crate::calls;
use crate::field_access;
use crate::scopes;

/// Resolve a span to its source file path relative to cwd, or None if not a real file.
fn resolve_source_key(
    tcx: TyCtxt<'_>,
    span: rustc_span::Span,
    cwd: &std::path::Path,
) -> Option<String> {
    let filename = tcx.sess.source_map().span_to_filename(span);
    let file_path = match &filename {
        FileName::Real(real) => real.local_path().map(|p| p.to_path_buf()),
        _ => None,
    }?;

    let abs_file =
        if file_path.is_absolute() { file_path } else { cwd.join(&file_path) };

    Some(
        abs_file
            .strip_prefix(cwd)
            .unwrap_or(&abs_file)
            .to_string_lossy()
            .to_string(),
    )
}

/// Properties that differ between standalone functions and methods.
struct FnProps {
    kind: UnitKind,
    params: usize,
    parent: Option<String>,
}

/// Extract metrics from a function/method body and build a Unit.
fn extract_fn_unit(
    tcx: TyCtxt<'_>,
    def_id: hir::def_id::LocalDefId,
    body_id: hir::BodyId,
    span: rustc_span::Span,
    source_key: &str,
    props: FnProps,
) -> Unit {
    let id = qualified_def_path_str(tcx, def_id.into());
    let lo = tcx.sess.source_map().lookup_char_pos(span.lo());
    let hi = tcx.sess.source_map().lookup_char_pos(span.hi());

    let body = tcx.hir_body(body_id);
    let branch_count = branches::count_branches(tcx, body);
    let max_scope = scopes::max_scope_lines(tcx, body);
    let (call_targets, calls_partial) =
        calls::extract_calls(tcx, def_id.to_def_id(), body);
    let (reads, writes) = field_access::extract_field_access(tcx, body);

    Unit {
        id,
        kind: props.kind,
        file: PathBuf::from(source_key),
        span: make_span(&lo, &hi),
        reads,
        writes,
        calls: call_targets,
        tags: vec![],
        params: props.params,
        branches: branch_count,
        max_scope_lines: max_scope,
        parent: props.parent,
        partial: calls_partial,
    }
}

/// Build a Unit for a struct definition.
fn extract_struct_unit(
    tcx: TyCtxt<'_>,
    def_id: hir::def_id::LocalDefId,
    span: rustc_span::Span,
    source_key: &str,
) -> Unit {
    let id = qualified_def_path_str(tcx, def_id.into());
    let lo = tcx.sess.source_map().lookup_char_pos(span.lo());
    let hi = tcx.sess.source_map().lookup_char_pos(span.hi());

    Unit {
        id,
        kind: UnitKind::Struct,
        file: PathBuf::from(source_key),
        span: make_span(&lo, &hi),
        reads: vec![],
        writes: vec![],
        calls: vec![],
        tags: vec![],
        params: 0,
        branches: 0,
        max_scope_lines: 0,
        parent: None,
        partial: false,
    }
}

/// Extract units from HIR for all source files in the crate.
///
/// Returns a map from relative source file path to the units found in that file.
pub fn extract_units(tcx: TyCtxt<'_>) -> HashMap<String, Vec<Unit>> {
    let mut results: HashMap<String, Vec<Unit>> = HashMap::new();
    let cwd = std::env::current_dir().unwrap_or_default();

    for item_id in tcx.hir_free_items() {
        let item = tcx.hir_item(item_id);
        let def_id = item.owner_id.def_id;
        let span = item.span;

        if span.from_expansion() || is_derived(tcx, def_id.to_def_id()) {
            continue;
        }

        let source_key = match resolve_source_key(tcx, span, &cwd) {
            Some(k) => k,
            None => continue,
        };

        let units = results.entry(source_key.clone()).or_default();

        match &item.kind {
            hir::ItemKind::Struct(_ident, _generics, _variant_data) => {
                units.push(extract_struct_unit(
                    tcx,
                    def_id,
                    span,
                    &source_key,
                ));
            }
            hir::ItemKind::Fn { sig, body: body_id, .. } => {
                let props = FnProps {
                    kind: UnitKind::Function,
                    params: count_params(sig.decl),
                    parent: None,
                };
                units.push(extract_fn_unit(
                    tcx,
                    def_id,
                    *body_id,
                    span,
                    &source_key,
                    props,
                ));
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
    let parent_id = resolve_impl_self_type(tcx, impl_block);

    for &impl_item_id in impl_block.items {
        let impl_item = tcx.hir_impl_item(impl_item_id);

        if impl_item.span.from_expansion() {
            continue;
        }

        let def_id = impl_item.owner_id.def_id;
        let span = impl_item.span;

        match &impl_item.kind {
            hir::ImplItemKind::Fn(sig, body_id) => {
                let props = FnProps {
                    kind: UnitKind::Method,
                    params: count_params_method(sig.decl),
                    parent: parent_id.clone(),
                };
                units.push(extract_fn_unit(
                    tcx, def_id, *body_id, span, source_key, props,
                ));
            }
            _ => {}
        }
    }
}

/// Resolve the self type of an impl block to a struct's def_path_str.
fn resolve_impl_self_type(
    tcx: TyCtxt<'_>,
    impl_block: &hir::Impl<'_>,
) -> Option<String> {
    if let hir::TyKind::Path(hir::QPath::Resolved(_, path)) =
        &impl_block.self_ty.kind
    {
        if let hir::def::Res::Def(_, def_id) = path.res {
            return Some(qualified_def_path_str(tcx, def_id.into()));
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

/// Check if a DefId or any of its ancestors is `#[automatically_derived]`.
///
/// This catches not just the top-level derive impl, but also nested items
/// generated inside it (e.g. serde's `__Visitor` struct and its trait impls).
fn is_derived(tcx: TyCtxt<'_>, mut def_id: rustc_hir::def_id::DefId) -> bool {
    loop {
        if tcx.is_automatically_derived(def_id) {
            return true;
        }
        match tcx.opt_parent(def_id) {
            Some(parent) => def_id = parent,
            None => return false,
        }
    }
}

/// Return a fully-qualified path string for a DefId, always including the crate name.
///
/// `def_path_str` omits the crate name for local items. This function
/// prepends it so that IDs are unambiguous across crates.
pub fn qualified_def_path_str(tcx: TyCtxt<'_>, def_id: DefId) -> String {
    let path = tcx.def_path_str(def_id);
    if def_id.krate == LOCAL_CRATE {
        let crate_name = tcx.crate_name(LOCAL_CRATE);
        format!("{crate_name}::{path}")
    } else {
        path
    }
}

fn make_span(lo: &rustc_span::Loc, hi: &rustc_span::Loc) -> Span {
    Span {
        start_line: lo.line,
        start_col: lo.col.0,
        end_line: hi.line,
        end_col: hi.col.0,
    }
}
