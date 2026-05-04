use mdlr_core::{Span, Unit, UnitKind};
use ra_ap_hir::{
    Adt, Crate, Function, HasSource, Impl, Module, ModuleDef, Semantics,
};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::ast::{self, HasAttrs, HasName};
use ra_ap_syntax::{AstNode, TextRange};
use ra_ap_vfs::Vfs;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::branches;
use crate::calls;
use crate::cognitive;
use crate::field_access;
use crate::path_util;
use crate::scopes;

/// A simple line index for mapping byte offsets to 1-based line/col positions.
struct LineIndex {
    line_starts: Vec<u32>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        Self { line_starts }
    }

    fn line_col(&self, offset: u32) -> (usize, usize) {
        match self.line_starts.binary_search(&offset) {
            Ok(line) => (line + 1, 0),
            Err(line) => {
                let line_start = self.line_starts[line - 1];
                (line, (offset - line_start) as usize)
            }
        }
    }
}

fn make_span(line_index: &LineIndex, range: TextRange) -> Span {
    let (start_line, start_col) =
        line_index.line_col(u32::from(range.start()));
    let (end_line, end_col) = line_index.line_col(u32::from(range.end()));
    Span { start_line, start_col, end_line, end_col }
}

/// Per-file context to avoid redundant parsing and line index computation.
struct FileContext {
    source_key: String,
    file_text: String,
    line_index: LineIndex,
}

impl FileContext {
    fn new(
        vfs: &Vfs,
        file_id: ra_ap_vfs::FileId,
        cwd: &std::path::Path,
    ) -> Option<Self> {
        let source_key = resolve_source_key(vfs, file_id, cwd)?;
        let file_text = get_file_text(vfs, file_id);
        let line_index = LineIndex::new(&file_text);
        Some(Self { source_key, file_text, line_index })
    }
}

/// Extract units from all source files in the target crates.
///
/// Returns a map from relative source file path to the units found in that file.
pub fn extract_units(
    db: &RootDatabase,
    sema: &Semantics<'_, RootDatabase>,
    vfs: &Vfs,
    target_crates: &[Crate],
    cwd: &std::path::Path,
) -> HashMap<String, Vec<Unit>> {
    let mut results: HashMap<String, Vec<Unit>> = HashMap::new();

    for krate in target_crates {
        extract_crate(db, sema, vfs, krate, cwd, &mut results);
    }

    results
}

fn extract_crate(
    db: &RootDatabase,
    sema: &Semantics<'_, RootDatabase>,
    vfs: &Vfs,
    krate: &Crate,
    cwd: &std::path::Path,
    results: &mut HashMap<String, Vec<Unit>>,
) {
    for module in krate.modules(db) {
        extract_module(db, sema, vfs, &module, cwd, results);
    }
}

fn extract_module(
    db: &RootDatabase,
    sema: &Semantics<'_, RootDatabase>,
    vfs: &Vfs,
    module: &Module,
    cwd: &std::path::Path,
    results: &mut HashMap<String, Vec<Unit>>,
) {
    // Process top-level declarations
    for def in module.declarations(db) {
        match def {
            ModuleDef::Function(func) => {
                extract_function(db, sema, vfs, &func, None, cwd, results);
            }
            ModuleDef::Adt(Adt::Struct(strukt)) => {
                extract_struct(db, vfs, &strukt, cwd, results);
            }
            _ => {}
        }
    }

    // Process impl blocks
    for impl_def in module.impl_defs(db) {
        extract_impl_block(db, sema, vfs, &impl_def, cwd, results);
    }
}

fn extract_function(
    db: &RootDatabase,
    sema: &Semantics<'_, RootDatabase>,
    vfs: &Vfs,
    func: &Function,
    parent_id: Option<String>,
    cwd: &std::path::Path,
    results: &mut HashMap<String, Vec<Unit>>,
) {
    let source = match func.source(db) {
        Some(s) => s,
        None => return,
    };

    let editioned_file_id = match source.file_id.file_id() {
        Some(id) => id,
        None => return, // macro-expanded, skip
    };
    let file_id = editioned_file_id.file_id(db);

    let fctx = match FileContext::new(vfs, file_id, cwd) {
        Some(f) => f,
        None => return,
    };

    let ast_fn = source.value;
    let body = match ast_fn.body() {
        Some(b) => b,
        None => return, // no body (e.g. trait declaration without default)
    };

    let fn_range = ast_fn.syntax().text_range();
    let is_method = parent_id.is_some();
    let id = if let Some(ref parent) = parent_id {
        path_util::qualified_method_path(
            db,
            ModuleDef::Function(*func),
            parent,
        )
    } else {
        path_util::qualified_path(db, ModuleDef::Function(*func))
    };
    let span = make_span(&fctx.line_index, fn_range);
    let kind = if is_method { UnitKind::Method } else { UnitKind::Function };

    let params = count_params(&ast_fn, is_method);
    let branch_count = branches::count_branches(&body);
    let cognitive_complexity = cognitive::compute_cognitive_complexity(&body);
    let max_scope = scopes::max_scope_lines(&body, &fctx.file_text);

    // For call resolution, we need AST nodes that are registered with Semantics.
    // sema.parse() gives us a tree that Semantics can look up; HasSource gives
    // us "detached" nodes that cause panics on resolve_method_call().
    // Parse the file through Semantics, then locate the matching fn body by range.
    let (call_targets, calls_partial) =
        extract_calls_via_sema(sema, db, editioned_file_id, fn_range);

    let param_names = extract_param_names(&ast_fn);
    let (reads, writes) =
        field_access::extract_field_access(&body, &param_names);

    let unit = Unit {
        id,
        kind,
        file: PathBuf::from(&fctx.source_key),
        span,
        reads,
        writes,
        calls: call_targets,
        tags: vec![],
        params,
        branches: branch_count,
        max_scope_lines: max_scope,
        parent: parent_id,
        cognitive_complexity,
        partial: calls_partial,
    };

    results.entry(fctx.source_key).or_default().push(unit);
}

/// Extract calls using Semantics-parsed AST nodes.
///
/// We parse the file through `sema.parse()` to get nodes that Semantics can
/// resolve, then find the function body by its text range.
fn extract_calls_via_sema(
    sema: &Semantics<'_, RootDatabase>,
    db: &RootDatabase,
    editioned_file_id: ra_ap_hir::EditionedFileId,
    fn_range: TextRange,
) -> (Vec<String>, bool) {
    let source_file = sema.parse(editioned_file_id);

    // Find the ast::Fn node at the same range in the Semantics-owned tree
    for node in source_file.syntax().descendants() {
        if let Some(sema_fn) = ast::Fn::cast(node) {
            if sema_fn.syntax().text_range() == fn_range {
                if let Some(body) = sema_fn.body() {
                    return calls::extract_calls(sema, db, &body);
                }
            }
        }
    }

    // Couldn't find the function in the Semantics tree — mark as partial
    (Vec::new(), true)
}

fn extract_struct(
    db: &RootDatabase,
    vfs: &Vfs,
    strukt: &ra_ap_hir::Struct,
    cwd: &std::path::Path,
    results: &mut HashMap<String, Vec<Unit>>,
) {
    let source = match strukt.source(db) {
        Some(s) => s,
        None => return,
    };

    let editioned_file_id = match source.file_id.file_id() {
        Some(id) => id,
        None => return,
    };
    let file_id = editioned_file_id.file_id(db);

    let fctx = match FileContext::new(vfs, file_id, cwd) {
        Some(f) => f,
        None => return,
    };

    let struct_range = source.value.syntax().text_range();
    let id =
        path_util::qualified_path(db, ModuleDef::Adt(Adt::Struct(*strukt)));
    let span = make_span(&fctx.line_index, struct_range);

    let unit = Unit {
        id,
        kind: UnitKind::Struct,
        file: PathBuf::from(&fctx.source_key),
        span,
        reads: vec![],
        writes: vec![],
        calls: vec![],
        tags: vec![],
        params: 0,
        branches: 0,
        max_scope_lines: 0,
        parent: None,
        cognitive_complexity: 0,
        partial: false,
    };

    results.entry(fctx.source_key).or_default().push(unit);
}

fn extract_impl_block(
    db: &RootDatabase,
    sema: &Semantics<'_, RootDatabase>,
    vfs: &Vfs,
    impl_def: &Impl,
    cwd: &std::path::Path,
    results: &mut HashMap<String, Vec<Unit>>,
) {
    // Skip auto-derived impls
    if is_derived_impl(db, impl_def) {
        return;
    }

    let parent_id = resolve_impl_self_type(db, impl_def);

    for item in impl_def.items(db) {
        if let ra_ap_hir::AssocItem::Function(func) = item {
            extract_function(
                db,
                sema,
                vfs,
                &func,
                parent_id.clone(),
                cwd,
                results,
            );
        }
    }
}

/// Resolve the self type of an impl block to a struct's qualified path.
fn resolve_impl_self_type(
    db: &RootDatabase,
    impl_def: &Impl,
) -> Option<String> {
    let self_ty = impl_def.self_ty(db);
    if let Some(adt) = self_ty.as_adt() {
        Some(path_util::qualified_path(db, ModuleDef::Adt(adt)))
    } else {
        None
    }
}

/// Check if an impl block is automatically derived (e.g. #[derive(...)]).
fn is_derived_impl(db: &RootDatabase, impl_def: &Impl) -> bool {
    let source = match impl_def.source(db) {
        Some(s) => s,
        None => return false,
    };

    // Macro-expanded impls from derive macros
    if source.file_id.file_id().is_none() {
        return true;
    }

    // Check for #[automatically_derived] attr on the impl
    for attr in source.value.attrs() {
        if let Some(path) = attr.path() {
            if path.syntax().text().to_string() == "automatically_derived" {
                return true;
            }
        }
    }

    false
}

/// Resolve a file_id to its source file path relative to cwd, or None if not a real file.
fn resolve_source_key(
    vfs: &Vfs,
    file_id: ra_ap_vfs::FileId,
    cwd: &std::path::Path,
) -> Option<String> {
    let vfs_path = vfs.file_path(file_id);
    let abs_path = vfs_path.as_path()?;
    let file_path: &std::path::Path = abs_path.as_ref();

    Some(
        file_path
            .strip_prefix(cwd)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string(),
    )
}

/// Get the text content of a file, reading from disk via the VFS path.
fn get_file_text(vfs: &Vfs, file_id: ra_ap_vfs::FileId) -> String {
    let vfs_path = vfs.file_path(file_id);
    if let Some(abs_path) = vfs_path.as_path() {
        let path: &std::path::Path = abs_path.as_ref();
        std::fs::read_to_string(path).unwrap_or_default()
    } else {
        String::new()
    }
}

/// Extract parameter names from a function signature (excluding `self`).
fn extract_param_names(func: &ast::Fn) -> Vec<String> {
    let param_list = match func.param_list() {
        Some(p) => p,
        None => return Vec::new(),
    };
    param_list
        .params()
        .filter_map(|p| {
            if let Some(ast::Pat::IdentPat(ident)) = p.pat() {
                ident.name().map(|n| n.text().to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Count parameters for a function, excluding self for methods.
fn count_params(func: &ast::Fn, is_method: bool) -> usize {
    let param_list = match func.param_list() {
        Some(p) => p,
        None => return 0,
    };

    let mut count = param_list.params().count();

    // If this is a method and has a self param, don't count it
    if is_method && param_list.self_param().is_some() {
        // self param is separate from params() iterator, so count is already correct
    } else if !is_method {
        // For standalone functions, also count self_param if present (unusual but possible)
        if param_list.self_param().is_some() {
            count += 1;
        }
    }

    count
}
