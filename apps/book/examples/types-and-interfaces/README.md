# Types and interfaces fixture

`types.flow` contains the central examples from FlowBook Chapter 7. The AST test in
`packages/ast/tests/book_examples.rs` keeps its interfaces, collection types, and quoted Struct
field access parseable and in canonical renderer form.

The fixture is intentionally a syntax and round-trip fixture. Catalog-aware reconciliation,
schema-pin inspection, and a captured runtime-drift failure still need to be rerun against the
named publication release before the chapter is marked final.
