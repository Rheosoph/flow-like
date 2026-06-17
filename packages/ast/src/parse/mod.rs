//! FlowScript front-end: `text -> BoardAst`.
//!
//! Layered like a small compiler front-end:
//! - [`lexer`] — context-free tokenizer.
//! - [`parser`] — recursive-descent + Pratt parser producing the [`crate::model`] IR.
//! - [`error`] — a position-carrying [`ParseError`].

mod error;
mod lexer;
mod parser;

pub use error::ParseError;
pub use parser::parse;
