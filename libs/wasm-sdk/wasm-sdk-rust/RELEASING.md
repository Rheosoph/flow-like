# Publish Rust SDK 0.4.0

Publish `flow-like-wasm-sdk` 0.4.0 before switching the Rust template to its
crates.io dependency. The macros crate is unchanged and remains at 0.3.7.

Run these commands from the Flow-Like repository root with its pinned Rust
toolchain. The SDK does not depend on Wasmtime; the runtime's Rust minimum is a
separate requirement. The SDK's `wit-bindgen` 0.53 dependency declares Rust 1.87
as its minimum. Release validation uses the repository toolchain, including for
the optional `rig` feature.

## Verify the release archive

Install the component target, then run the SDK tests with and without Rig:

```bash
rustup target add wasm32-wasip2
cargo test --manifest-path libs/wasm-sdk/wasm-sdk-rust/Cargo.toml --locked --target host-tuple
cargo test --manifest-path libs/wasm-sdk/wasm-sdk-rust/Cargo.toml --locked --all-features --target host-tuple
```

Review the [release notes](CHANGELOG.md) and package contents. The list must
include `src/resources.rs` and `wit/flow-like-node.wit`. Cargo flattens the WIT
symlink into the archive, so a consumer does not need its source elsewhere in
this repository. [Cargo packaging reference](https://doc.rust-lang.org/cargo/commands/cargo-package.html)

```bash
cargo package --manifest-path libs/wasm-sdk/wasm-sdk-rust/Cargo.toml --locked --allow-dirty --list
cargo publish --manifest-path libs/wasm-sdk/wasm-sdk-rust/Cargo.toml --locked --allow-dirty --all-features --dry-run --registry crates-io
cargo publish --manifest-path libs/wasm-sdk/wasm-sdk-rust/Cargo.toml --locked --allow-dirty --all-features --target wasm32-wasip2 --dry-run --registry crates-io
```

These dry runs build the packaged source without uploading. `--allow-dirty`
lets you validate the reviewed changes before committing them. For publication,
commit the release files first, or deliberately add `--allow-dirty` to publish
the reviewed working tree. Cargo otherwise rejects uncommitted package changes.
[Cargo publish reference](https://doc.rust-lang.org/cargo/commands/cargo-publish.html)

## Authenticate and publish

Use a crates.io account with permission to publish `flow-like-wasm-sdk`. If
Cargo is not already authenticated, run this in your own terminal and enter
your token at its prompt:

```bash
cargo login --registry crates-io
```

Publish only the SDK. This command uploads the version permanently; it cannot
be overwritten. [Cargo publishing guide](https://doc.rust-lang.org/cargo/reference/publishing.html)

```bash
cargo publish --manifest-path libs/wasm-sdk/wasm-sdk-rust/Cargo.toml --locked --all-features --registry crates-io
```

Wait for the registry entry to become available, then verify its exact version:

```bash
cargo info flow-like-wasm-sdk@0.4.0 --registry crates-io
```

If Cargo reports an index timeout after upload, check the published version
before attempting another upload. Once available, update the SDK index at
`libs/wasm-sdk/README.md` to mark 0.4.0 as published.

## Switch the template after publication

In `templates/wasm-node-rust/Cargo.toml`, replace the SDK dependency and its local
path comments with:

```toml
flow-like-wasm-sdk = { version = "0.4.0", features = ["rig"] }
```

Keep `rig` enabled because the template includes agent examples. Remove the
README instructions that require a sibling SDK checkout and replace its
repository-relative SDK links with public links. Regenerate the lockfile and
build both the native tests and component against the published crate:

```bash
cargo update --manifest-path templates/wasm-node-rust/Cargo.toml -p flow-like-wasm-sdk --precise 0.4.0
cargo test --manifest-path templates/wasm-node-rust/Cargo.toml --locked --target host-tuple
cargo build --manifest-path templates/wasm-node-rust/Cargo.toml --locked --release --target wasm32-wasip2
```

The SDK entry in the template lockfile must have a registry source and checksum.
Copy the template directory outside this repository and repeat its tests and
component build to verify that cloning it requires no sibling SDK directory.
Commit the template manifest, lockfile, and documentation changes together.
