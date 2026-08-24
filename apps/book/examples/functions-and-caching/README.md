# Functions and caching fixture

`functions.flow` is the compact Chapter 12 example. It separates a pull-evaluated pure helper from
a cached reusable Function layer, calls both functions through their first-parameter method form,
wraps the Function in an explicitly registered Event handler for agent use, and invalidates the
shared cache namespace from a separate mutation boundary. The cached resolver is deterministic
but structurally impure because its `if` creates execution flow; caching is safe only because its
body has no side effects and all freshness dependencies are explicit inputs.

The `directoryRevision` input keeps concurrent old computations on an old key. The invalidation
Event stands in for the successful end of a system-directory update. Invalidating after every read
would defeat the cache; invalidation belongs after the authoritative data changes.

`packages/ast/tests/book_examples.rs` keeps the source parseable and in canonical renderer form.
Before publication, the example must also be reconciled and executed against the matching catalog
to verify the cache miss, hit, expiry, and namespace-invalidation paths.
