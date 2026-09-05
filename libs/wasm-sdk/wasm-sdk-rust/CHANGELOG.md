# Changelog

## 0.4.0

Nodes in a package can pass handles to arbitrary Rust objects retained within
one run. The `resources` module adds `insert`, `with`, `with_mut`, `remove`, and
`close`, with type checks and conflicting-borrow errors. Objects need no
serialization, `Send`, or `Sync` implementation. Different package instances,
runs, and security domains remain separate.

The Rust template demonstrates retained buffers, iterators, and WASI TCP
listeners and connections through the same registry. Guest networking code
requires compatible WASI network grants and the runtime's address policy. The
runtime releases these resources when the run ends, including failure and
cancellation.

### Migrating from 0.3.7

The native `rig` feature now exposes Rig 0.38.2 instead of 0.34.0. Its public
types have changed, including
`FlowLikeCompletionModel::StreamingResponse`, which is now
`rig_provider::FlowLikeStreamingResponse`. Code that names Rig types or
constructs Rig messages directly may need changes. This public API change is
why the release advances to 0.4.0. The unchanged macros crate stays at 0.3.7.

Components using the registry require a runtime that implements
`metadata.new-resource-handle`. Retained objects also require reusable node
exports and run-owned package instances. Command-style `wasi:cli/run`
execution still starts with fresh guest memory for each command.

Run teardown releases guest memory and host resources without running guest
Rust destructors. Call `resources::close` during execution to run `Drop`, or
`remove` to perform a client's graceful shutdown. Stored handles cannot restore
objects in later runs. Retaining a client grants no extra network permissions
and does not drive its guest event loop between calls.

The SDK archive includes its WIT definitions, so consumers need no Flow-Like
checkout. After publication, the Rust template can use the registry dependency
shown in the [publishing guide](RELEASING.md#switch-the-template-after-publication).
