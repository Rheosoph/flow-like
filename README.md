<p align="center">
  <a href="https://flow-like.com">
    <img src="apps/desktop/public/app-logo.webp" alt="Flow-Like" width="72" />
  </a>
</p>

<h1 align="center">Flow-Like</h1>

<p align="center">
  <strong>Build Apps around typed Flows in FlowScript or on a live canvas.</strong><br/>
  Keep executable logic with its interfaces, data, packages, access, releases, and run evidence.
</p>

<p align="center">
  <a href="https://flow-like.com"><img src="https://img.shields.io/badge/website-flow--like.com-0a7cff" alt="Website" /></a>
  <a href="https://docs.flow-like.com"><img src="https://img.shields.io/badge/docs-read-0a7cff" alt="Documentation" /></a>
  <a href="https://discord.com/invite/mdBA9kMjFJ"><img src="https://img.shields.io/discord/673169081704120334?label=Discord&color=5865F2" alt="Discord" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-BSL%201.1-blue" alt="BSL 1.1 license" /></a>
  <img src="https://img.shields.io/github/stars/Rheosoph/flow-like.svg?style=flat&amp;label=stars&amp;color=f5b400&amp;cacheSeconds=3600" alt="GitHub stars" />
</p>

<p align="center">
  <a href="https://app.flow-like.com">Try online</a> ·
  <a href="https://flow-like.com/download">Download Studio</a> ·
  <a href="https://docs.flow-like.com">Documentation</a> ·
  <a href="https://book.flow-like.com">FlowBook</a> ·
  <a href="https://discord.com/invite/mdBA9kMjFJ">Discord</a>
</p>

---

Most software work starts in the middle. The database already exists. The API has quirks that
never made it into the docs. A useful change still has to fit that system and remain understandable
after it ships.

Flow-Like is a source-available developer platform for building and running application logic. A
**Flow** is one executable process, and its persisted graph is the **Board** that the Rust runtime
executes locally or on configured infrastructure.

An **App** is the unit a team owns and ships. It keeps Flows beside the Events and Pages that expose
them, the data they use, reusable packages, members, roles, and release settings. An App Event
selects a Flow entry, version, and execution location for an API, schedule, chat, form, Page, REST
endpoint, MCP server, or another supported caller.

**Studio** is the complete desktop application. Developers use it to manage Apps, edit Flows,
inspect data and packages, run logic locally, and trace results back to the Board. The browser app
and configured backend provide shared access and remote execution.

<p align="center">
  <img src="apps/website/src/images/parallax/workflow-core.png" alt="Flow-Like Studio showing FlowScript and the matching Flow canvas side by side." width="100%" />
</p>

## Text and canvas edit the same Flow

FlowScript is the typed text form of a Flow. The canvas shows the same Flow as nodes, pins, and
connections. Edits from either view update its Board.

This entry receives an incident report and chooses the log path:

```ts
use log::*

eventsGeneric triageIncident(payload: Struct, report: string) {
    const normalized = report.trim()
    if (normalized.contains({ substring: "production is on hold", ignoreCase: true })) {
        error({ message: normalized, toast: false })
    } else {
        info({ message: normalized, toast: false })
    }
}
```

<p align="center">
  <a href="./apps/book/src/assets/workflows/incident-triage.webp">
    <img src="apps/book/src/assets/workflows/incident-triage.webp" alt="The Incident Triage Flow on the Studio canvas as Generic Event, Trim String, Contains, Branch, Log Error, and Print Info nodes." width="100%" />
  </a>
</p>
<p align="center"><sub>Generated from the <a href="./apps/book/examples/incident-triage/triage.flow">checked-in FlowScript</a> with the real reconciler and Studio auto-layout.</sub></p>

The canvas exposes the same rule as six nodes. The `trim` and `contains` calls resolve to catalog
nodes, and their values travel over typed wires into a Branch node. FlowScript earns its keep
during broad edits and code review. Open the canvas to trace a branch or inspect a failed run.

Studio renders the current Board as canonical FlowScript. When you apply a source edit, Studio
parses it, checks it against the node catalog, and writes the change back as Board commands. The
Rust runtime executes that Board.

<details>
<summary>See how a source edit becomes Board commands</summary>
<br/>
<p align="center">
  <img src="apps/book/src/assets/authoring-roundtrip.svg" alt="The canvas and FlowScript edit the same typed Board model before the Rust runtime executes a selected version." width="100%" />
</p>
</details>

Read [FlowBook](https://book.flow-like.com) for the language concepts, worked examples, and current
round-trip boundaries.

## Run Flow-Like

| Online | Desktop | Self-hosted | Source |
| --- | --- | --- | --- |
| [Open the web app](https://app.flow-like.com) | [Download Studio](https://flow-like.com/download) for macOS, Windows, or Linux | [Run the Compose stack](https://docs.flow-like.com/self-hosting/docker-compose/installation/) | Build the current `dev` branch with the steps below |

### Build from source

Install Git, [mise](https://mise.jdx.dev/), [Tauri 2 system
dependencies](https://v2.tauri.app/start/prerequisites/), `protoc`, and a C/C++ toolchain.
Run these commands from the repository root:

```bash
git clone --branch dev https://github.com/Rheosoph/flow-like.git
cd flow-like
mise trust
mise install
bun install
cp apps/desktop/.env.example apps/desktop/.env
mise run dev:desktop
```

`mise install` supplies the toolchain declared in [`mise.toml`](./mise.toml).
`mise run dev:desktop` detects the current platform and starts Studio.

Useful tasks:

```bash
mise tasks
mise run dev:web
mise run dev:docs
mise run dev:book
mise run check
mise run fix
```

The detailed setup guide lives at
[docs.flow-like.com/dev/build](https://docs.flow-like.com/dev/build/).

### Self-host with Docker Compose

The checked-in [Compose directory](./apps/backend/docker-compose/) builds the browser app, API
gateway and API replicas, Rust runtime workers, WASM compiler, realtime signaling, server-side
Event services, PostgreSQL, Redis, and database initialization on one host. Studio remains the
desktop application and can connect to that backend.

<p align="center">
  <img src="apps/docs/src/assets/DockerComposeArchitecture.svg" alt="The Docker Compose stack connects browser and desktop clients to the web app, API, execution workers, persistence, collaboration, and optional monitoring services." width="100%" />
</p>

Start with these files:

| File | Purpose |
| --- | --- |
| [`docker-compose.yml`](./apps/backend/docker-compose/docker-compose.yml) | Service topology, health checks, ports, volumes, and optional monitoring profile |
| [`.env.example`](./apps/backend/docker-compose/.env.example) | Image, URL, identity, storage, replica, and signing-key settings |
| [`flow-like.config.example.json`](./apps/backend/docker-compose/flow-like.config.example.json) | Hub identity provider, domains, feature flags, legal links, and Event sinks |
| [`monitoring/`](./apps/backend/docker-compose/monitoring/) | Prometheus, Grafana, Tempo, exporters, dashboards, and rules |

The documented installation path is:

```bash
git clone --branch dev https://github.com/Rheosoph/flow-like.git
cd flow-like/apps/backend/docker-compose
cp .env.example .env
cp flow-like.config.example.json flow-like.config.json

../../../tools/gen-execution-keys.sh --export
# Add the generated keys, OIDC settings, public URLs, and storage configuration to .env.
# Point FLOW_LIKE_CONFIG and FLOW_LIKE_RUNTIME_CONFIG_FILE at flow-like.config.json.

docker compose config --quiet
docker compose up -d --build
docker compose ps --all
```

The stack expects external object storage and does not create its buckets or containers. The
copied environment template currently selects AWS, while the stock API image omits the AWS
runtime-credential feature. Select a provider supported by that image or rebuild the API target
with the required feature. Read the complete
[Docker Compose installation guide](https://docs.flow-like.com/self-hosting/docker-compose/installation/)
before exposing the stack publicly. The optional `monitoring` profile adds Prometheus, Grafana,
Tempo, and PostgreSQL and Redis exporters.

## Runtime requirements stay with the Flow

Before a run starts, Flow-Like's pre-run analysis walks the complete Flow and reports the runtime
variables, OAuth requirements, local-only nodes, and WebAssembly permissions it finds.

| Runtime question | Where the answer lives |
| --- | --- |
| Where does this entry run? | An App Event selects Local or Remote execution. |
| Which logic is live? | The Event points to a Board entry and version. |
| What changes by environment? | Runtime values and Event overrides provide configured input. |
| What may the code access? | Nodes and packages declare capabilities; credentials are scoped separately. |

The App Event and Board modes currently select local or remote execution. Pre-run reports when any
node, including one inside a nested layer, requires local access. The current dispatcher does not
yet reject every incompatible remote selection from that aggregate flag. A single run remains in
one environment, and device-local secret values stay outside the Board and remote payloads.

A Board can run in Studio or on configured remote runtimes. The maintained self-hosting guides
cover Docker Compose and Kubernetes. Backend directories also contain deployment work for AWS,
Azure, and GCP; check each target's documentation and status before relying on equivalent behavior.

## Start with the system you have

Typed catalog nodes connect Flows to the services and data already in use, including local files
and devices. Reusable packages put a domain operation behind declared inputs and outputs. The
current system can keep owning its data while a Flow validates input or coordinates calls.

Start with the integration that costs the team the most time. If it fails, run evidence points
back to a node on the Board. Capability declarations show what that operation needs from the host.

App Events expose a pinned Flow entry to callers without duplicating its logic. Pages and APIs can
reuse the same Flow, while model calls remain explicit typed nodes with node-attributed run
evidence. Supported models can run locally when their runtime and files are installed, or through
a configured provider.

<p align="center">
  <img src="apps/docs/src/assets/FlowLikeAppAnatomy.svg" alt="A Flow-Like App groups Flows with experiences, data, reusable building blocks, access, and releases." width="100%" />
</p>

## Find your way around the repository

| Area | Start here |
| --- | --- |
| FlowScript model, parser, renderer, and linting | [`packages/ast`](./packages/ast/) |
| Lowering, reconciliation, and guarded Apply | [`packages/core/src/flow/ast`](./packages/core/src/flow/ast/) |
| Canvas and FlowScript editor | [`packages/ui/components/flow`](./packages/ui/components/flow/) |
| Studio desktop application | [`apps/desktop`](./apps/desktop/) |
| Rust runtime and execution | [`packages/core`](./packages/core/), [`packages/executor`](./packages/executor/) |
| Built-in capabilities and integrations | [`packages/catalog`](./packages/catalog/) |
| Local and hosted deployment shapes | [`apps/backend`](./apps/backend/) |

Flow-Like is a Rust and TypeScript monorepo. Bun manages the JavaScript workspace, Tauri hosts the
desktop client, and the runtime and core application model live in Rust.

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/Rheosoph/flow-like)

## Extend the platform

The catalog turns each supported operation into a node with typed pins. For domain-specific work,
add a native node or package the operation as a WebAssembly component with declared host
capabilities. The [templates](./templates/) cover fifteen source languages; host API support
varies by language and template maturity.

Applications can also control the platform through the
[TypeScript SDK](https://www.npmjs.com/package/@flow-like/sdk) and
[Python SDK](https://pypi.org/project/flow-like/).

## License

Flow-Like is source-available under the [Business Source License 1.1](./LICENSE). The Additional
Use Grant permits use that does not compete with Flow-Like or substantially similar Rheosoph
products. Entities with more than 2,000 employees or more than €300 million in annual revenue
need a commercial license. For each version, MPL 2.0 takes effect on the earlier of the stated
eight-year Change Date or the fourth anniversary of that version's first public distribution.

## Contributing

Code and documentation contributions are welcome.

- Browse [good first issues](https://github.com/Rheosoph/flow-like/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22).
- Open pull requests against `dev` and run the checks relevant to your change.
- Use [GitHub Discussions](https://github.com/Rheosoph/flow-like/discussions) or
  [Discord](https://discord.com/invite/mdBA9kMjFJ) when you want to talk through an approach.

<a href="https://github.com/Rheosoph/flow-like/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=Rheosoph/flow-like" alt="Flow-Like contributors" />
</a>

---

<p align="center">
  <a href="https://flow-like.com">Website</a> ·
  <a href="https://docs.flow-like.com">Documentation</a> ·
  <a href="https://book.flow-like.com">FlowBook</a> ·
  <a href="./LICENSE">License</a> ·
  <a href="./CODE_OF_CONDUCT.md">Code of Conduct</a> ·
  <a href="./SECURITY.md">Security</a>
</p>

<p align="center"><sub>Built in Munich, Germany.</sub></p>
