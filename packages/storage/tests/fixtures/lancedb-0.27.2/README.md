These fixtures were written by LanceDB Rust **0.27.2**, Lance **4.0.0**, and Arrow **57.2.0**. The regression test copies each fixture to a temporary directory before opening, querying, appending, or maintaining it.

`legacy.lance` reproduces Flow-Like's old creation path, including its V2.2 and append write options. LanceDB 0.27.2 overrides the per-write storage version with its connection setting during table creation, so this fixture's physical data file uses **V2.0**. The footer records `(0, 3)`, Lance's encoding for V2.0.

`../lancedb-0.27.2-v2.2/legacy.lance` uses the same records and indexes with a connection-level V2.2 setting. Its data footer records `(2, 2)`. This second fixture covers older tables that explicitly enabled that format.

Each table contains four rows and three B-tree indexes, on `id`, `event_date`, and `occurred_at`. It has no vector index. The vector values exercise compatibility of the saved vector data and schema.

| Column | Arrow type | Values |
|--------|------------|--------|
| `id` | Non-null `Int64` | 1, 2, 3, 4 |
| `title` | Non-null `Utf8` | alpha, beta, gamma, delta |
| `event_date` | Nullable `Date32` | 2025-01-01, 2025-01-02, 2025-01-03, null |
| `occurred_at` | Nullable `Timestamp(Millisecond, UTC)` | Midnight UTC on those dates, then null |
| `vector` | Non-null `FixedSizeList<Float32, 4>` with nullable items | [1,0,0,0], [0,1,0,0], [0,0,1,0], [0,0,0,1] |

The checked-in `generate.rs` and `generate.Cargo.toml` are the source and dependency manifest used to generate this table. To regenerate it, copy the manifest to a new temporary directory as `Cargo.toml` and copy the source to `src/main.rs` within that directory. Run:

```sh
cargo run --manifest-path /path/to/temporary-generator/Cargo.toml -- /path/to/new-output
```

Pass `v2.2` after the output directory to create the second fixture with an explicit connection-level format setting. The output directory must not already contain a table named `legacy`. Review the generated files before replacing this fixture. Copy them with `cp -R` or `rsync -a`. `.pre-commit-config.yaml` excludes this directory from `trailing-whitespace`: pre-commit classifies `.manifest` as text, and the hook deletes length bytes that precede `0x0A` inside Lance's binary manifests. Keep the generator on the old crate versions so the fixture continues to test reads of data written before the upgrade.
