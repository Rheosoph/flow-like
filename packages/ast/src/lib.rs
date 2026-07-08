//! FlowScript: the text-domain IR for flow-like boards.
//!
//! This crate is the *language half* of the Board ⇄ AST ⇄ Text pipeline described in
//! `todo/ast.md`. It is intentionally free of any dependency on the `flow-like` core graph
//! model so it compiles fast and can later back a standalone linter / language server.
//!
//! - [`model`] — the [`BoardAst`] IR types.
//! - [`render`] — [`BoardAst`] → FlowScript text.
//! - [`parse`] — FlowScript text → [`BoardAst`].
//! - [`signatures`] — node signature stubs (the ~1200-function problem).
//! - [`text`] — pure text helpers (casing, quoting).
//!
//! Lowering (`Board → BoardAst`) and reconcile (`BoardAst → commands`) live in `flow-like`
//! core, which depends on this crate.

pub mod model;
pub mod parse;
pub mod render;
pub mod schema;
pub mod signatures;
pub mod text;

pub use model::*;
pub use parse::{ParseError, parse};
pub use render::{RenderOptions, render, render_type_ref};
pub use schema::{
    apply_interface_schemas, interface_name_for_schema, interfaces_for_variables, normalize_schema,
    schema_from_interface, schema_from_interface_with_defs,
};
pub use signatures::{
    DeclarationFile, NodeSchemas, SIGNATURE_SET_VERSION, SigParam, Signature, SignatureSet,
    declarations_by_category, declarations_by_package, render_signatures, schema_sidecar,
};
pub use text::{is_valid_identifier, quote_string, to_camel_case};
