# Working on catalog crates

Check the crate that owns the nodes you are changing. Each implementation crate
has its own compiler invocation, so independent domains can compile together and
an edit can reuse the other domains' artifacts. Applications continue to use the
existing catalog facades and product feature bundles.

| Facade | Implementation crates |
| --- | --- |
| `flow-like-catalog-std` | `std-ui`, `std-values`, `std-numbers`, `std-text`, `std-runtime` |
| `flow-like-catalog-data` | GitHub, Google, Microsoft, Atlassian, Notion, Databricks, LinkedIn; remaining data nodes in the facade |
| `flow-like-catalog-web` | Telegram, Discord, mail; HTTP and transports in the facade |
| `flow-like-catalog-media` | Audio, document, image, video; bit nodes in the facade |

The shortened names in the table have the `flow-like-catalog-` package prefix.
Data service crates use `flow-like-catalog-data-<service>`, and web/media children
use the corresponding `web-` or `media-` prefix.

```bash
cargo check -p flow-like-catalog-std-text --features execute
cargo test -p flow-like-catalog-std --test registry_compatibility
cargo check -p flow-like-catalog-data-github --features execute
cargo check-catalog-desktop
```

Enable `execute` when checking an execution implementation. Metadata-only checks
do not type-check code behind that feature. Product bundles still select the
complete node set and the runtime capabilities appropriate to the product.

## Shared types and dependencies

Use support crates for shared implementations rather than importing another
node collection:

- `flow-like-catalog-std-support` owns utility scores, value normalization, and
  date decoding shared by standard node domains.
- `flow-like-catalog-data-support` owns cache, query session, path, remote-service,
  graph, and chat attachment support used by multiple catalogs.
- `flow-like-catalog-embedding` owns embedding handles shared by LLM nodes and
  WASM host functions.
- `flow-like-catalog-media-support` owns the media provider credential check.

Compatibility modules re-export the original types. Define each cache type in
one place: independently recreated structs would break runtime downcasts even
if their fields were identical. Put an external dependency on the implementation
crate that uses it, and forward execution features from the facade.

`flow-like-runtime` provides `NodeLogic`, board types, and the concrete execution
context. Catalog manifests use the dependency name `flow-like` as an alias for
this package, preserving imports inside node implementations. The public
`flow-like` facade combines runtime with `flow-like-editor`; application code can
keep using that facade. A production node crate must depend on the runtime
directly to avoid waiting for editor and copilot compilation.

The engine also uses `flow-like-a2ui-schema` and `flow-like-editor-contracts`
for types that can compile independently of the application runtime.

## Registration

Each implementation crate exports `NodeLogic` and `register_node` at its root and
calls `flow_like_catalog_build_helper::generate_with_paths("src")` in `build.rs`.
The generated registry includes source paths. Facades combine the registries
with a stable `Path` comparison to preserve the original traversal order,
including multiple registrations in one source file.

Keep the original paths below each child's `src` directory when moving nodes.
Move the physical files out of the facade: the scanner walks files and does not
interpret module declarations or `cfg` attributes. Re-exports alone cannot
prevent stale registrations. Existing supported registration markers are
`#[register_node]` and `#[crate::register_node]`; changing that syntax affects
discovery and needs a separate catalog compatibility check.

The compatibility tests freeze public node paths and registration order. Run
them after an extraction, and update the fixtures deliberately when adding or
removing nodes. Run feature-boundary checks to prevent a helper import from
reintroducing a dependency on an entire node collection:

```bash
./tools/check-feature-boundaries.sh --mode deps --offline \
  catalog-std-execute catalog-llm-execute wasm-host
```

## Compile measurements

The timing runner creates new target directories without cleaning existing
artifacts. Check and full-build measurements are separate:

```bash
./tools/compile-times.sh desktop-std-string
./tools/compile-times.sh --command build desktop-std-string
./tools/compile-times.sh --command build --profile ci --incremental 0 backend-executor
```

Keep the toolchain, target, features, profile, job limit, and cache policy fixed
when comparing runs. The timestamp-based incremental phase measures a dirty
source file; use actual code edits as well when evaluating a development loop.
