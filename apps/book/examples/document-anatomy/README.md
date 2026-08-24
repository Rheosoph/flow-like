# Document anatomy fixture

`anatomy.flow` is the complete source example from FlowBook Chapter 6. The AST test in
`packages/ast/tests/book_examples.rs` keeps it parseable and in canonical renderer form.

The fixture intentionally exercises every top-level document section: `use` declarations,
interfaces, variables and decorators, a Function layer, and an Event entry block.
