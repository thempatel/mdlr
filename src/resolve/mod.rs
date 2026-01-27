//! Rust dependency resolution for local crates.
//!
//! This module provides name resolution for Rust code within a Cargo workspace,
//! including:
//! - Workspace member crates
//! - Path dependencies
//! - Module hierarchy
//! - Use statement imports
//!
//! # Example
//!
//! ```ignore
//! use mdlr::resolve::{CargoWorkspace, ResolutionContext};
//!
//! // Discover the workspace
//! let workspace = CargoWorkspace::discover(".")?;
//!
//! // Build the resolution context
//! let ctx = ResolutionContext::build(workspace);
//!
//! // Resolve a name
//! let resolved = ctx.resolve("HashMap", "my_crate", &["crate", "module"]);
//! ```
//!
//! # Limitations
//!
//! - No macro expansion: macro-generated code is not visible
//! - No external crates: dependencies from crates.io are marked as unresolved
//! - No type inference: method calls through trait impls can't be resolved
//! - No const evaluation: `include!()` and similar won't work
//! - Glob imports are best-effort

mod cargo;
mod modules;
mod resolve;
mod uses;

// Re-export main types
pub use cargo::{CargoWorkspace, CrateInfo};
pub use modules::{ItemDef, ItemKind, ItemSpan, ModuleGraph, ModuleNode, ModulePath};
pub use resolve::{ResolutionContext, ResolvedPath};
pub use uses::{UseKind, UseStatement, Visibility};
