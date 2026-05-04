use ra_ap_hir::db::HirDatabase;
use ra_ap_hir::ModuleDef;

/// Return a fully-qualified `crate_name::path` string for a ModuleDef.
///
/// `canonical_path` omits the crate name, so we prepend it to match the
/// old `tcx.def_path_str(def_id)` output that always included the crate.
pub fn qualified_path(db: &dyn HirDatabase, def: ModuleDef) -> String {
    let krate = match def.module(db) {
        Some(m) => m.krate(),
        None => return format!("{def:?}"),
    };
    let edition = krate.edition(db);
    let crate_name = krate
        .display_name(db)
        .map(|dn| dn.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    match def.canonical_path(db, edition) {
        Some(path) => format!("{crate_name}::{path}"),
        None => {
            if let Some(name) = def.name(db) {
                format!("{crate_name}::{}", name.as_str())
            } else {
                crate_name
            }
        }
    }
}

/// Build a qualified path for a method in an impl block.
///
/// `canonical_path` for methods gives `module::method_name` without the
/// impl's self type. This function builds `crate::module::Type::method_name`
/// by using the parent type path we already resolved.
pub fn qualified_method_path(
    db: &dyn HirDatabase,
    def: ModuleDef,
    parent_path: &str,
) -> String {
    if let Some(name) = def.name(db) {
        format!("{parent_path}::{}", name.as_str())
    } else {
        qualified_path(db, def)
    }
}
