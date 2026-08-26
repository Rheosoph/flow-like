# State and secrets fixture

`state.flow` is the compact Chapter 11 example. It separates shared App configuration from a
local client-profile/device secret and shows the current `@readonly` metadata explicitly.

The omission of `lastSuccessfulSync` and reference data is deliberate: those values belong in a
cache, App Storage, or a database according to their required lifetime and access pattern—not in
run-local Flow variables.

`packages/ast/tests/book_examples.rs` keeps the source parseable and in canonical renderer form.
Before publication, the example must also be reconciled against the matching LLM catalog and its
local and remote missing-credential paths must be exercised separately.
