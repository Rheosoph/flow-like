# Wasm runtime upgrades

Wasmtime 48.0.1 requires Rust 1.95.0. Build the API, compilation workers, and
executors with the same Wasmtime major version. A worker rejects compilation
jobs whose target key names a different version, so a worker running version 48
cannot publish its artifacts under a `wt47` key.

Precompiled `.cwasm` artifacts from Wasmtime 47 cannot run on Wasmtime 48. The
registry and local caches derive their version from the workspace dependency;
this upgrade changes their platform keys to `wt48`, such as
`linux-x86_64-wt48` and `ios-pulley64-wt48`. Source `.wasm` packages keep their
existing node ABI and do not need recompilation for this runtime upgrade.

During deployment:

1. Coordinate the API, compiler, and executor upgrade. If `EXECUTOR_PLATFORM`
   is configured outside the repository, update its version suffix to `wt48`
   while retaining the executor's OS and architecture.
2. Regenerate active packages' Linux artifacts through
   `POST /admin/packages/ensure-wasm-artifacts`. Use the registry's package
   recompilation operation for any other required targets.
3. Wait for the required targets to finish compiling before routing runs to
   the upgraded executors. The API requires the current target's artifacts
   before dispatching a package.

Local caches discard incompatible artifacts and compile the original Wasm
again when compilation is enabled. Executors also fall back to verified source
Wasm when a downloaded artifact cannot be deserialized. Deployments that disable
compilation need matching `wt48` artifacts available in advance.

Wasmtime 48 also denies TCP and UDP socket creation by default. Flow-Like enables
each protocol according to the package's granted capabilities and applies the
existing host allowlist and execution-environment policy. HTTP host interfaces
retain their separate capability checks.

See the upstream [48.0.0 release notes](https://github.com/bytecodealliance/wasmtime/releases/tag/v48.0.0)
for the Rust requirement and changed WASI defaults, and the
[48.0.1 fixes](https://github.com/bytecodealliance/wasmtime/releases/tag/v48.0.1)
for component context slots and WASIp2 HTTP request headers.
