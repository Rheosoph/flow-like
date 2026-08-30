# Control-flow fixture

`control.flow` is the compact Chapter 10 example. It covers a Boolean branch, a nested
`else if` ladder in canonical form, a named execution-arm block, sequential and parallel
collection loops, a bounded `while`, a final function return, and an Event result.

`packages/ast/tests/book_examples.rs` keeps the source parseable and in canonical renderer form.
Before publication, the example must also be reconciled with the release catalog, executed, and
captured in both authoring views. The HTTP call is illustrative and must use a controlled test
endpoint when that runtime capture is made.
