//! FlowScript: the text-domain IR for flow-like boards.
//!
//! This crate is the *language half* of the Board ⇄ AST ⇄ Text pipeline described in
//! `todo/ast.md`. It is intentionally free of any dependency on the `flow-like` core graph
//! model so it compiles fast and can later back a standalone linter / language server.
//!
//! - [`model`] — the [`BoardAst`] IR types.
//! - [`render`] — [`BoardAst`] → FlowScript text.
//! - [`parse`] — FlowScript text → [`BoardAst`].
//! - [`redact`] — strip declared values and long literals before a source is stored off-machine.
//! - [`naming`] — `namespace::alias` derivation for nodes and the name-collision contract.
//! - [`signatures`] — node signature stubs (the ~1200-function problem).
//! - [`text`] — pure text helpers (casing, quoting).
//!
//! Lowering (`Board → BoardAst`) and reconcile (`BoardAst → commands`) live in `flow-like`
//! core, which depends on this crate.

pub mod model;
pub mod naming;
pub mod parse;
pub mod redact;
pub mod render;
pub mod schema;
pub mod signatures;
pub mod text;

pub use model::*;
pub use naming::{
    CollisionKind, EffectiveNames, NAME_OVERRIDES, NAMESPACES, NameCollision, NameEntry,
    NameFields, NameOverride, NamespaceSpec, NodeNames, VALUE_TYPE_NAMESPACES, check_names,
    default_receiver_pin, derive_alias, derive_namespace, effective_names, effective_receiver_pin,
    effective_spelling, is_keyword, is_value_type_namespace, legacy_display,
    namespace_accepts_receiver, qualified_name, receiver_class, receiver_class_of, schema_title,
};
pub use parse::{ParseError, parse};
pub use redact::{MAX_LITERAL_CHARS, MAX_SOURCE_CHARS, RedactedFlowScript, redact_flowscript};
pub use render::{RenderOptions, render, render_template, render_type_ref};
pub use schema::{
    apply_interface_schemas, interface_name_for_schema, interfaces_for_variables, normalize_schema,
    schema_from_interface, schema_from_interface_with_defs,
};
pub use signatures::{
    DeclarationFile, NodeSchemas, SIGNATURE_SET_VERSION, SigParam, Signature, SignatureSet,
    declarations_by_category, declarations_by_package, is_signature_line, render_signatures,
    schema_sidecar,
};
pub use text::{is_valid_identifier, quote_string, to_camel_case};
