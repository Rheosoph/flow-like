<p align="center">
  <a href="https://flow-like.com">
    <img src="apps/desktop/public/app-logo.webp" alt="Flow-Like" width="72" />
  </a>
</p>

<h1 align="center">Flow-Like</h1>

<p align="center">
  <strong>One local-first platform for workflows, data, AI, and apps.</strong><br/>
  Build end-to-end use cases visually, keep terabytes of data in-platform, and run<br/>
  anywhere — laptop, server, phone, or edge. Offline by default, cloud-ready when you need it.
</p>

<p align="center">
  <a href="https://flow-like.com"><img src="https://img.shields.io/badge/website-flow--like.com-0a7cff" alt="Website" /></a>
  <a href="https://docs.flow-like.com"><img src="https://img.shields.io/badge/docs-read-0a7cff" alt="Docs" /></a>
  <a href="https://discord.com/invite/mdBA9kMjFJ"><img src="https://img.shields.io/discord/673169081704120334?label=Discord&color=5865F2" alt="Discord" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-BSL%201.1-blue" alt="License" /></a>
  <img
    src="https://img.shields.io/github/stars/Rheosoph/flow-like.svg?style=flat&amp;label=stars&amp;color=f5b400&amp;cacheSeconds=3600"
    alt="GitHub stars"
  />
</p>

<p align="center">
  <a href="https://app.flow-like.com">Try online</a> ·
  <a href="https://flow-like.com/download">Download</a> ·
  <a href="https://docs.flow-like.com">Docs</a> ·
  <a href="https://flow-like.com/compare">Compare</a> ·
  <a href="https://discord.com/invite/mdBA9kMjFJ">Discord</a>
</p>

---

Flow-Like is a **source-available, local-first platform for building end-to-end use cases** — **workflows, data, AI, and apps in one runtime**. Instead of stitching together an automation tool (n8n, Zapier), an app builder (Retool), an AI framework, and separate data infrastructure, you build the whole thing in one place — with a **native data layer that holds terabytes in-platform**, a **fully typed execution engine**, and complete lineage on every run. Runs on your laptop, server, phone, or edge. No forced cloud, no black boxes.

<p align="center">
  <img src="assets/recording.gif" alt="Flow-Like Visual Workflow Studio" width="100%" />
</p>

<p align="center">
  <strong>1,500+ nodes</strong> · <strong>15 languages</strong> · runs offline · terabytes in-platform · <strong>zero external services</strong>
</p>

## What you can build

- **A private RAG assistant** over your own PDFs and files — local embeddings and a local LLM, no API keys, nothing leaving your machine.
- **Data pipelines at scale** — ingest, transform, and store terabytes in-platform, then expose the result as an app.
- **Agentic automations** — scrape, enrich, and act across APIs, databases, Slack, and email, on a schedule or a webhook.
- **Internal & customer-facing apps** — a custom UI backed by your workflows and built-in storage, shipped to desktop, web, or edge.

## Try it in under a minute

| Try online | Desktop app | From source |
|:--|:--|:--|
| [**app.flow-like.com**](https://app.flow-like.com) — no install | [**Download**](https://flow-like.com/download) for macOS · Windows · Linux | [Build it yourself](#build-from-source) |

## Why Flow-Like

**Runs where you choose — not where a vendor tells you.**
Your laptop, a private server, an air-gapped factory network, a Raspberry Pi, or cloud you control. Your data stays where you put it. Local-first by default; deploy to your own AWS/GCP/Azure when you're ready. No vendor lock-in, no "upgrade to enterprise for basic autonomy."

**Fast enough to run on a phone.**
The engine is written in Rust — native code, no garbage collector, no interpreter overhead. That's what makes real workflows viable on constrained devices, and millions of executions trivial on a server.

| | Flow-Like | Typical Node/Python engines |
|--|--|--|
| Throughput | ~244,000 workflows/sec | ~200 workflows/sec |
| Latency | ~0.6 ms | ~50–500 ms |

**Fully typed, fully traceable.**
Every workflow tracks *what data flows where* and *why*. Type-safe input/output contracts catch errors before deployment, and every input, transformation, and output is recorded with complete lineage — replay any run. View the same workflow three ways: **Process** for the business, **Data** for analysts, **Execution** for engineers.

**AI-native, and it runs on-device.**
LLMs, embeddings, vision object detection, speech-to-text, and text-to-speech all run locally — no API keys, no data leaving the machine — or in the cloud when you'd rather. Every model call is logged with full context (inputs, outputs, model version, reasoning trace), with approval gates and guardrails on every step.

**Your data lives in the platform — not bolted on.**
Most tools make you wire up an external database, object store, and warehouse before you can do anything real. Flow-Like ships a **native data layer**: files, tables, and hybrid keyword + vector search on the canvas, with **unlimited payload sizes and terabytes of data held right where your workflows run** — no external S3, Postgres, or warehouse to provision, no glue code, no egress surprises. Share datasets, apps, and templates across your team from the same system.

**Ship apps, not just automations.**
Package workflows as installable apps with built-in storage and a no-code UI builder. Run them offline, share them, or embed the whole editor white-labeled in your own product.

## How it compares

Flow-Like isn't "a better n8n." Most stacks split the **data, automation, AI, and app layers** across separate products and leave you to integrate them. Flow-Like is a platform for building **end-to-end use cases** — data included — in one governed, high-performance runtime.

| Layer | The usual stack | In Flow-Like |
|--|--|--|
| **Data & storage** | S3 + Postgres + a warehouse | **Native data layer — terabytes in-platform, unlimited payloads, hybrid search, no external infra** |
| Workflows & automation | Zapier · n8n · Make | Built in, typed, and replayable |
| AI & agents | LangChain · Dify · CrewAI | On-device + cloud, with guardrails |
| Apps & UI | Retool · Appsmith | Ship desktop, web & mobile apps |
| Governance | bolted on, per tool | RBAC, audit trail & lineage across all of it |

The result: **one platform instead of five tools plus the glue between them** — running on your hardware, offline-capable, and shareable across your team.

> Full head-to-head against 21 tools (n8n, Retool, Temporal, LangChain, Airflow, Palantir Foundry, UiPath, and more) → **[flow-like.com/compare](https://flow-like.com/compare)**

## Licensing — the honest version

Flow-Like is **source-available under the Business Source License (BSL 1.1)**, not an OSI open-source license *yet*. Here's exactly what that means so there are no surprises:

- ✅ **Free for the vast majority of users.** You can use it in production for free if your organization has **fewer than 2,000 employees and under €300M in annual revenue**. That covers virtually every startup, SMB, and team.
- ✅ **Read, fork, modify, self-host, and contribute** — the full source is here, and non-production use is always free.
- ⏳ **It becomes open source on a timer.** Each release automatically re-licenses under the **[MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/)** on its Change Date. What ships today is guaranteed to become true open source.
- 💼 **Two things need a commercial license:** (1) building a product that directly competes with Flow-Like, and (2) large entities (2,000+ employees **or** €300M+ revenue).

We chose BSL to keep Flow-Like sustainable and independent while still shipping all the source and a guaranteed path to open source. Questions or a commercial license? [Reach out](https://flow-like.com) or read the [full license](./LICENSE).

## What's inside

- **Visual Workflow Studio** — a no-code IDE with type-aware wiring, live validation, inline execution feedback, and snapshot debugging.
- **1,500+ built-in nodes** — APIs & webhooks, databases, files (Excel/CSV/PDF), on-device AI (LLMs, vision object detection, speech-to-text, text-to-speech, embeddings), messaging (Slack, Discord, email), IoT, logic, auth — [browse the catalog](https://docs.flow-like.com/).
- **Built-in data layer** — object storage, tables, and hybrid keyword + vector search on the canvas. Terabytes in-platform, unlimited payloads, shareable across your team — no external services.
- **Write nodes in 15 languages** — custom nodes run as sandboxed WebAssembly, so extend the catalog in Rust, TypeScript, Python, Go, C#, Java, Swift, and more. [Templates here](./templates/).
- **SDKs** — control workflows programmatically from [Node.js/TypeScript](https://www.npmjs.com/package/@flow-like/sdk) (`npm i @flow-like/sdk`) or [Python](https://pypi.org/project/flow-like/) (`uv add flow-like`), with optional LangChain integration. [SDK docs](https://docs.flow-like.com/dev/sdks/overview).
- **White-label** — themes, brand design tokens, SSO (OIDC/JWT), per-tenant usage metering, and a headless engine. Embed the editor in your own product.

## Build from source

```bash
# Prerequisites: mise, Rust, Bun, Protobuf compiler, Tauri prerequisites
git clone https://github.com/Rheosoph/flow-like.git
cd flow-like
mise trust && mise install   # toolchains: Rust, Bun, Node, Python, uv
bun install                  # Node packages
mise run dev:desktop         # start the desktop app
```

Every dev/build/deploy task lives in [`mise.toml`](./mise.toml). Run `mise tasks` to list them, e.g. `mise run dev:web`, `mise run build:desktop`, `mise run check`. Platform-specific notes are in the [docs](https://docs.flow-like.com/contributing/getting-started/).

## Contributing

Flow-Like is built in the open and we'd love your help — new nodes, bug fixes, docs, themes, or ideas.

- 🌱 **[Good first issues](https://github.com/Rheosoph/flow-like/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)** — a friendly place to start.
- 📖 **[CONTRIBUTING.md](./CONTRIBUTING.md)** — setup and guidelines.
- 💬 **[Discord](https://discord.com/invite/mdBA9kMjFJ)** — questions, help, and discussion.

If Flow-Like is useful to you, **[star the repo](https://github.com/Rheosoph/flow-like)** — it genuinely helps others find the project.

### Contributors

Every avatar here shipped something. Yours could be next.

<a href="https://github.com/Rheosoph/flow-like/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=Rheosoph/flow-like" alt="Flow-Like contributors" />
</a>

<p align="center">
  <a href="https://star-history.com/#Rheosoph/flow-like&Date">
    <img src="https://api.star-history.com/svg?repos=Rheosoph/flow-like&type=Date" alt="Star history chart" width="70%" />
  </a>
</p>

---

<details>
<summary><strong>Screenshots</strong></summary>
<br/>

<p align="center"><img src="https://cdn.flow-like.com/website/SelectYourModel.webp" alt="AI Model Catalog" width="100%" /></p>
<p align="center">
  <img src="https://cdn.flow-like.com/website/CreateApp.webp" alt="Create Apps" width="48%" />
  <img src="https://cdn.flow-like.com/website/Store.webp" alt="Template Store" width="48%" />
</p>
<p align="center"><img src="https://cdn.flow-like.com/website/Storage.webp" alt="Built-in storage and search" width="100%" /></p>
<p align="center"><img src="https://cdn.flow-like.com/website/RightsAndRoles.webp" alt="Rights and roles" width="100%" /></p>

</details>

<details>
<summary><strong>FAQ</strong></summary>
<br/>

**Is it free?** For almost everyone, yes — see [Licensing](#licensing--the-honest-version). Under 2,000 employees and €300M revenue: free in production.

**Can I run it fully offline?** Yes, 100% — ideal for air-gapped and secure environments. Switch to online mode anytime to collaborate.

**Can I embed it in my product?** Yes. Flow-Like is white-label ready — embed the editor, theme it, wire up SSO, or run the engine headlessly.

**What languages do I write?** The builder is no-code. Custom nodes run as sandboxed WebAssembly, so you can write them in **15 languages** — Rust, TypeScript, Python, Go, C#, Java, Kotlin, Swift, and more (see [`templates/`](./templates/)). SDKs (TS/Python) and REST APIs cover programmatic control.

**Is it production-ready?** It's actively developed and used in production. Test thoroughly for mission-critical workflows; see [releases](https://github.com/Rheosoph/flow-like/releases).

**How do I get support?** [Discord](https://discord.com/invite/mdBA9kMjFJ), [Docs](https://docs.flow-like.com), or [GitHub Issues](https://github.com/Rheosoph/flow-like/issues).

</details>

<details>
<summary><strong>Built with</strong></summary>
<br/>

**Runtime:** [Rust](https://github.com/rust-lang/rust) · [Tauri](https://github.com/tauri-apps/tauri) · [Tokio](https://github.com/tokio-rs/tokio) · [Axum](https://github.com/tokio-rs/axum) · [SeaORM](https://github.com/SeaQL/sea-orm)
**AI/ML:** [llama.cpp](https://github.com/ggerganov/llama.cpp) · [Candle](https://github.com/huggingface/candle) · [ONNX Runtime](https://github.com/microsoft/onnxruntime)
**Frontend:** [Next.js](https://github.com/vercel/next.js) · [React Flow](https://github.com/xyflow/xyflow) · [shadcn/ui](https://github.com/shadcn-ui/ui) · [Tailwind CSS](https://github.com/tailwindlabs/tailwindcss)
**Tooling:** [mise](https://github.com/jdx/mise) · [Bun](https://github.com/oven-sh/bun) · [Biome](https://github.com/biomejs/biome)

Thank you to every maintainer and contributor of these projects.

</details>

---

<p align="center">
  <a href="https://flow-like.com">Website</a> ·
  <a href="https://docs.flow-like.com">Docs</a> ·
  <a href="https://flow-like.com/blog">Blog</a> ·
  <a href="./LICENSE">License</a> ·
  <a href="./CODE_OF_CONDUCT.md">Code of Conduct</a> ·
  <a href="./SECURITY.md">Security</a>
</p>

<p align="center"><sub>Made with care in Munich, Germany.</sub></p>
