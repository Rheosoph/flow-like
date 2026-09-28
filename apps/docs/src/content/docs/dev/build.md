---
title: Building from Source
description: Set up the repository and build the Flow-Like desktop application
sidebar:
  order: 10
---

## Clone the development branch

The repository's default branch is `dev`. The `alpha` branch is used for
release snapshots; do not assume a `main` branch exists.

```bash
git clone https://github.com/Rheosoph/flow-like.git
cd flow-like
git checkout dev
```

Fork the repository first if you plan to [contribute](/dev/contribute/).

## Install the toolchain

Flow-Like uses [mise](https://mise.jdx.dev/) to pin and run the repository
toolchain. The root `mise.toml` currently installs Rust, Bun, Node.js 22,
Python 3.12, and uv.

Host builds use `beta-2026-09-27` (Rust 1.99.0-beta.8, LLVM 23.1.1). This
dated prerelease includes the [LLVM fix for nondeterministic loop optimization](https://github.com/llvm/llvm-project/pull/188821)
needed for byte-identical optimized rebuilds. Rust 1.97.1 and 1.98.1 still
produce differing executable instructions in the regression case.

`rust-toolchain.toml` controls CI and Docker builders across platforms. Mise
and Xcode use matching pins, checked by CI. Use that pin for local builds too.
Compiler updates must pass the clean rebuild regression on Linux, macOS, and
Windows for x64 and ARM64; standalone releases also compare two complete
binaries. Keep these checks when moving to a stable compiler. Guest WASM
packages document their compiler requirements in their templates.

```bash
mise trust
mise install
bun install
```

The desktop build also needs:

- the [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/)
  for your operating system;
- `protoc`, because Rust build scripts compile the repository's protobuf
  definitions;
- a working C/C++ build toolchain for native dependencies.

Platform-specific native libraries and mobile toolchains have additional
requirements. Use the task you intend to run as the final source of truth.

## Run the desktop app

The top-level task detects the current operating system and architecture:

```bash
mise run dev:desktop
```

Use an explicit task only when you need to override that detection:

```bash
mise run dev:desktop:mac:arm
mise run dev:desktop:mac:intel
mise run dev:desktop:win:x64
mise run dev:desktop:win:arm
mise run dev:desktop:linux:x64
mise run dev:desktop:linux:arm
```

To run the desktop app with the local API and runtime:

```bash
mise run dev:desktop:local
```

## Build a release bundle

```bash
mise run build:desktop
```

Tauri writes binaries and installers below `target/release/`; the precise
bundle directory and extension depend on the platform.

### Maintain desktop release tooling

The frontend release job installs the full workspace and exports its assets
for the native targets. Native jobs install the smaller package set in
`.github/release-tools` into the runner's temporary directory.

Keep its Tauri CLI and dotenv CLI versions aligned with the versions resolved
in the root `bun.lock`. Update the release-tools lockfile in a temporary
directory outside the repository so Bun does not discover the monorepo
workspace. Commit that directory's `package.json` and `bun.lock` together.

The [native release workflow](https://github.com/Rheosoph/flow-like/blob/dev/.github/workflows/release-native.yml)
runs `apps/desktop/scripts/sync-version.ts` explicitly. It does not install the
desktop workspace, so it cannot rely on the desktop postinstall script to
synchronize versions.

## Other useful tasks

```bash
mise tasks
mise run dev:web
mise run dev:docs
mise run build:web
mise run build:docs
mise run check
mise run fix
```

`mise.toml` is the authoritative task list. Several tasks wrap package-local
scripts, so run them from the repository root unless a page explicitly says
otherwise.

## Linux rendering issues

Tauri uses the system WebKitGTK stack on Linux. If a window fails to render,
confirm the Tauri prerequisites for your distribution and check the
[upstream Tauri issues](https://github.com/tauri-apps/tauri/issues) for your
WebKitGTK or graphics-driver combination.
