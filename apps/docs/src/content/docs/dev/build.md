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

Host builds use Rust 1.97.1, pinned in `rust-toolchain.toml`, mise, CI, and
Docker builders. This satisfies Wasmtime 48's [Rust 1.95 minimum](https://github.com/bytecodealliance/wasmtime/blob/v48.0.1/Cargo.toml).
Use the repository pin when building Flow-Like. Guest WASM packages have
separate compiler requirements documented in their templates.

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
