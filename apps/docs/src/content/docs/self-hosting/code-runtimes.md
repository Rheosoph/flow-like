---
title: Code runtimes for bots
description: How bots run Python and JavaScript in a sandbox, which runtime files are downloaded, and how to mirror them on your own CDN or ship them offline
---

Bots with the code execution capability run programs in a WebAssembly sandbox. No
interpreter is bundled in the app or the server image: each one is a pinned file that
is downloaded on first use, checked against a SHA-256 hash, precompiled for the host
and cached.

## Languages and packages

| Language | Runtime | Download | Notes |
|---|---|---|---|
| `python` | CPython 3.12 for WASI (VMware Wasm Labs build) | 25 MB | Standard library only. |
| `javascript` | QuickJS-NG 0.18.0 for WASI | 1.5 MB | ES2023, top-level `await`, no Node or browser APIs, no npm. |
| `python` with `packages` | wasi-python 0.1.2 (CPython 3.15) | 94 MB (wheel); 150 MB on disk | numpy, pandas, scipy, scikit-learn, statsmodels, matplotlib, pillow, sympy, networkx, openpyxl, lxml and more, linked into the interpreter. Used only when a bot asks for `packages`. |

Every run gets a fresh instance whose only visible directory is `/flow`: input files
under `/flow/in`, the working directory `/flow/out`. Files written to `/flow/out`
come back to the bot as workspace files; png, jpg, gif and webp files are also shown
to the model. The network is off unless the owner approves named hosts. Wall time and
memory are limited per run (server: 120 s and 512 MiB by default; the scientific
build raises the memory floor to 1 GiB).

### Licences

CPython: PSF. QuickJS-NG: MIT. wasi-python: Apache-2.0. The scientific build carries
the licence texts of everything linked into it (numpy, pandas, scipy, scikit-learn,
matplotlib, OpenBLAS, FreeType, HarfBuzz and others, all permissive) in the wheel's
`LICENSES` folder.

## Where the files come from

Each runtime file is tried at these URLs in order, and nothing is installed unless its
size and SHA-256 match the values pinned in the app:

| Runtime | Mirror URL | Fallback |
|---|---|---|
| Python | `https://cdn.flow-like.com/runtimes/python/3.12.0-20231211-040d5a6/python.wasm` | GitHub release of `vmware-labs/webassembly-language-runtimes` |
| QuickJS | `https://cdn.flow-like.com/runtimes/quickjs/0.18.0/qjs-wasi.wasm` | GitHub release v0.18.0 of `quickjs-ng/quickjs` |
| Scientific Python | `https://cdn.flow-like.com/runtimes/python-scientific/0.1.2/wasi_python-0.1.2-py3-none-any.whl` | `files.pythonhosted.org` |

The mirror URLs are served by the Flow-Like CDN. The fallbacks are the upstream
projects; a missing mirror object (404) falls through to them.

### Cache layout

Downloads live under `<cache dir>/sandbox`, where the cache dir is the platform cache
directory, or `CACHE_DIR` when set (on Lambda it is `/tmp`):

```text
sandbox/artifacts/python/<version>/python.wasm
sandbox/artifacts/quickjs/<version>/qjs-wasi.wasm
sandbox/artifacts/python-scientific/<version>/python.wasm
sandbox/artifacts/python-scientific/<version>/lib/python315.zip
sandbox/aot/modules/<hash>-<os>-<arch>-wt48.cwasm      precompiled for this host
```

Point `CACHE_DIR` at a persistent volume on servers that restart often, so the
download and the precompile (about a minute for the scientific build) happen once.

## Mirror the files yourself

To serve the files from your own storage:

1. Download each file from the fallback URL and check its hash against the pinned
   value (`libs/nodes/code-interpreter/src/sandbox/artifact.rs`):
   `shasum -a 256 qjs-wasi.wasm`.
2. Upload it to your bucket under the same relative path as the table above.
3. Pre-seed instead of mirroring: copy the files into the cache layout above before
   the first run. The app finds a file whose size and hash match and never goes to
   the network, so air-gapped hosts work. For the scientific build, extract
   `wasi_python/artifacts/python.wasm` to `python.wasm` and
   `wasi_python/artifacts/python-stdlib.zip` to `lib/python315.zip` from the wheel.

A mirror can change where files come from, not which files run: the hashes are
compiled into the app. Moving to a new interpreter version needs an app release.

## Server configuration

Bots on the server run code on the executor. The API offers `code_run` to a bot only
when an executor takes tool jobs, for the languages in `EXECUTOR_TOOL_CODE_LANGUAGES`.
When the variable is unset the API offers `python,javascript`, which is right for
every executor image built with the sandbox (`server`, `executor` and
`server-execute` bundles). Set it to a subset (`python`) to restrict languages, or to
an empty value to turn `code_run` off. The executor itself answers a call for a
language it cannot run with a clear error.

## iOS

iOS forbids generating native code, so the sandbox needs interpreters precompiled to
Pulley bytecode. Until the app ships and loads those, iOS reports no languages and
bots are not offered `code_run`. The release step builds one `.cwasm` per runtime:

```bash
cargo run -p flow-like-catalog-code-interpreter --features execute \
  --example precompile_ios -- python.wasm python-ios.cwasm
```

The example uses the engine settings the on-device sandbox must use (fuel metering,
iOS memory layout, Wasmtime 48). Publish the output next to the matching `.wasm`; the
cache key is the hash of the `.wasm` plus `ios-pulley64-wt48`.
