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

Prepare the desktop environment file from the repository root:

```bash
cp -n apps/desktop/.env.example apps/desktop/.env
```

Keep an existing `.env` if you already have one. The example sets
`NEXT_PUBLIC_API_URL` to the hosted API and uses `tauri://localhost/callback`
and `tauri://localhost/logout` for the desktop OAuth redirects. Review those
values for your hub before running the app. The macOS ARM task sources this
file before starting Tauri.

The desktop build also needs:

- the [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/)
  for your operating system;
- `protoc`, because Rust build scripts compile the repository's protobuf
  definitions;
- a working C/C++ build toolchain for native dependencies;
- on Linux, the unixODBC driver manager that ODBC nodes link
  (`sudo apt install unixodbc-dev`, Fedora `unixODBC-devel`). macOS desktop builds
  compile and bundle their own copy; macOS executor builds use Homebrew's
  (`brew install unixodbc`). Windows uses the system ODBC library.

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

To run the desktop app with the local API and runtime, first complete the
[native local backend setup](/self-hosting/local-development/), including its
database, storage, identity, and signing configuration. Then run:

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

## Build and upgrade the file editors

Web and desktop development and build scripts prepare the pinned Storytold
editor runtimes automatically. Development starts the app immediately and prepares
missing runtimes in the background. Progress and failures are logged in
`target/craft-viewers/development/build.log`. Production builds wait for all
runtimes. The first run downloads their source and builds
it with Rust's `wasm32-unknown-unknown` target and the matching `wasm-bindgen`
CLI. This is a substantial build. Later runs reuse verified assets from
`target/wordcraft` and `target/craft-viewers`. Generated files in each app's
`public/wordcraft` and `public/craft` directories are excluded from Git.

The suite builds up to four editors in parallel, allowing one build per four
available CPU threads and 4 GiB of memory, with a minimum of one build. Each build
uses up to four Cargo jobs by default. Set `CRAFT_BUILD_CONCURRENCY` to choose
the number of simultaneous editors, or `CARGO_BUILD_JOBS` to override the jobs
per editor. For example, `CRAFT_BUILD_CONCURRENCY=2 bun run build:craft-viewers`
builds two at a time; use `1` for sequential builds. These settings also apply
to web and desktop builds and background development builds. Automatic
concurrency accounts for a positive `CARGO_BUILD_JOBS` override; a relative
negative job count keeps automatic concurrency at one.

Concurrent editors share a versioned `wasm-bindgen` installation and use separate
Cargo build directories so Cargo's build lock does not serialize them. The first
parallel run recompiles dependencies and uses more disk space than the former
shared directory. Sequential suites reuse that shared directory. A failed editor
stops the remaining queue; active builds finish before the suite reports failure.

The browser loads only the selected viewer. PDFs and common media files use
native previews and load their editing runtime after **Edit**. The
container builds prepare these same assets before building the Next.js export.
WASM payloads are compressed and split into files below the static hosting size
limit; the host verifies their hashes before loading them.

Audio, Slides and Video use browser/OS codecs for compressed media. Audio imports
decode through Web Audio into PCM. Slides use browser media elements for embedded
audio and video. Video uses browser media decoding and WebCodecs encoders for MP4
export. Codec availability depends on the browser or desktop system webview. An
unsupported format produces an error and leaves the document open; these editors
have no bundled software codec fallback. PCM handling and lossless file writers
remain in Rust.

Each integration keeps its source pin and adapter separate from the upstream
repository. WordCraft lives in `packages/wordcraft`; the other adapters live in
`packages/craft-viewers/<runtime>`. To upgrade one, update its `upstream.json`
with the exact commit, workspace version and `wasm-bindgen` version from the
upstream `Cargo.lock`. Review changes to its readers and writers before changing
the format registry in `packages/ui/components/storage/craft-viewers.ts`.

```bash
# Build the complete suite, or just one runtime.
bun run build:craft-viewers
node packages/craft-viewers/build-runtime.mjs gridcraft

# Validate storage controls, routing and the iframe protocol.
bun run test:craft-viewers
bun run test:wordcraft

# Exercise the actual WASM readers and writers in Chrome.
bun tests/craft-browser/verify.ts
bun tests/craft-office-browser/verify.ts
bun tests/craft-media-browser/verify.ts
bun tests/craft-image-browser/verify.ts
bun tests/craft-solid-browser/verify.ts

# Discover live MCP tools and verify document edits and undo.
bun tests/craft-mcp-browser/verify.ts

# Create blank documents, export, save and reopen them in each editor.
bun tests/craft-create-browser/verify.ts

# Verify builder sizing and editor messaging inside responsive previews.
CRAFT_NESTED_PREVIEW=1 bun tests/craft-create-browser/verify.ts solidcraft wordcraft
bun tests/a2ui-file-editor-browser/verify.ts
```

The live editor MCP adapters reuse each pin's tool definitions and handlers.
WordCraft, DeckCraft, GridCraft, VectorCraft, DesignCraft, SolidCraft, CADCraft
and SoundCraft link their upstream MCP crates through `live-mcp.mjs`. PDFCraft
runs its upstream automation server against the UI's engine session. PhotoCraft
and FilmCraft couple their servers to native transports; `rmcp-live.mjs` takes
the parameter types, tool attributes and bridge handlers from the pinned source
and supplies an in-process browser bridge. No tool catalog is maintained in the
agent node.

When upgrading, check changes to MCP schemas, control methods and undo behavior
alongside the readers and writers. The shared `mcp-policy.rs` limits operations
to the hosted document. File lifecycle operations stay with Flow-Like, and native
screenshots and window operations are not exposed. New upstream tools and control
methods may need an adapter review before they can be used in the browser. Run
the MCP tests and browser checks after each upgrade.

The adapters apply checked patches to known integration points. Builds fail if
those points have changed, so upgrades require review. `CRAFT_SOURCE_GRIDCRAFT`
(and the equivalent variable for another runtime) can select a local checkout
at the pinned revision. Builds rewrite the adapter-owned integration files in
that checkout. Leave it unset for an isolated checkout under `target`.

The media adapters also check the selected WASM dependency graph with
`packages/craft-viewers/codec-policy.mjs`. Keep the software codec exclusion when
upgrading. If an adapter needs Cargo to reconcile removed dependency edges, its
`reconcileLock` option permits only package versions and checksums already in the
upstream pin's lockfile.

Each runtime includes a license page at `/craft/<runtime>/licenses/index.html`
or `/wordcraft/licenses/index.html`, available from its **About** button. Keep
the complete `licenses` directory with the runtime in both web and desktop
distributions. It contains dependency and asset notices, license texts, and
exact MPL source archives as `.crate` files (gzip-compressed tar archives).
Desktop source links use the native Save dialog to copy these bundled archives
to a user-selected folder. They work without fetching source from an upstream site.
MPL-covered files retain those terms; other files keep their stated licenses.
Flow-Like's license does not restrict rights granted by third-party licenses.

Normal builds run `cargo tree --offline --locked --target wasm32-unknown-unknown`
with normal dependency edges and the runtime's build feature flags. The generated
`licenses/manifest.json` records the resolved dependencies, selected licenses,
source locations and registry source checksums. Reviewed SPDX license choices
live in `packages/craft-viewers/licenses.mjs`. When a package omits notices,
`packages/craft-viewers/license-data/overrides.json` supplies them for an exact
`package@version`, with pinned upstream sources. It also records obligations
disclosed in upstream notices that are missing from Cargo's license field.

Builds stop for missing or unreviewed license expressions, missing notice texts,
unavailable checksum-verified MPL archives, or local changes to MPL dependencies.
To ship a modified MPL dependency, extend the packaging to include its modified
source; the current check rejects it instead of publishing the original archive.
Review dependency and embedded-asset changes during each upgrade, including
changes that pass these checks. Update license choices or pinned notice overrides
only after reviewing their terms, then rebuild and check the generated page and
source downloads. These checks cover license packaging; they do not establish
codec patent clearance.

Install the test browser with `bunx puppeteer browsers install chrome`, or set
`CHROME_PATH` to an existing Chrome executable. Video browser tests also require
`ffmpeg` and `ffprobe` on `PATH`. Review representative saved files
in their original applications after an upgrade, including complex formatting,
formulas, layers and codec-dependent media. Browser smoke tests establish that
imports and exports run; they do not establish complete format fidelity.

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
