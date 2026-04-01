<p align="center">
  <a href="https://flow-like.com">
    <img src="apps/desktop/public/app-logo.webp" alt="Flow-Like Logo" width="80" />
  </a>
</p>
<h1 align="center">Flow-Like</h1>
<p align="center">
  <strong>If you can't see it, you can't trust it.</strong><br/>
  A Rust-powered workflow engine that runs on your device — laptop, server, or phone.<br/>
  Fully typed. Fully traceable. Fully yours.
</p>
<p align="center">
  <a href="https://app.codacy.com/gh/TM9657/flow-like/dashboard?utm_source=gh&utm_medium=referral"><img src="https://app.codacy.com/project/badge/Grade/244d2db2a84f4e79b64d984639a2b18f" alt="Codacy Badge" /></a>
  <a href="https://discord.com/invite/mdBA9kMjFJ"><img src="https://img.shields.io/discord/673169081704120334" alt="Discord" /></a>
  <a href="https://app.fossa.com/projects/custom%2B49014%2Fflow-like?ref=badge_small"><img src="https://app.fossa.com/api/projects/custom%2B49014%2Fflow-like.svg?type=small" alt="FOSSA Status" /></a>
  <a href="https://flow-like.com"><img src="https://img.shields.io/badge/website-flow--like.com-0a7cff?logo=google-chrome&logoColor=white" alt="Website" /></a>
  <a href="https://docs.flow-like.com"><img src="https://img.shields.io/badge/docs-docs.flow--like.com-0a7cff?logo=readthedocs&logoColor=white" alt="Docs" /></a>
  <a href="https://flow-like.com/download"><img src="https://img.shields.io/badge/download-Desktop%20App-28a745?logo=tauri&logoColor=white" alt="Download" /></a>
</p>
<p align="center">
  <a href="https://github.com/TM9657/flow-like/stargazers"><strong>⭐ Star on GitHub</strong></a> ·
  <a href="https://docs.flow-like.com"><strong>📖 Docs</strong></a> ·
  <a href="https://discord.com/invite/mdBA9kMjFJ"><strong>💬 Discord</strong></a> ·
  <a href="https://flow-like.com/download"><strong>📥 Download</strong></a>
</p>

---

Flow-Like is a **visual workflow automation platform** that runs entirely on your hardware. Build workflows with drag-and-drop blocks, run them on your laptop, phone, or server, and get a clear record of where data came from, what changed, and what came out — no cloud dependency, no black boxes, no guesswork.

<p align="center">
  <img src="assets/recording.gif" alt="Flow-Like Visual Workflow Studio" width="100%" />
</p>

---

## Why Flow-Like?

### You decide where your workflows run. Not us.

Most workflow tools force you into their cloud. Your data leaves your machine, passes through third-party servers, and you're locked into their infrastructure. Offline? Stuck. Want to self-host? Pay enterprise prices.

Flow-Like runs **wherever you choose** — your laptop, your phone, your private server, or cloud infrastructure you control. Your data stays where you put it. No forced cloud dependency, no vendor lock-in, no "upgrade to enterprise for basic autonomy."

**Local-first by default. Cloud-ready when you need it.**

Automate on your couch without Wi-Fi. Deploy to your own AWS/GCP/Azure. Run in air-gapped factory networks. Process sensitive data in hospital environments. Your workflows, your infrastructure, your rules.

### A Rust engine fast enough to run on a phone

The reason this works is raw performance. Flow-Like's engine is built in Rust — compiled to native code, no garbage collector, no runtime overhead. The same workflow that takes 500ms in a Node.js engine takes **0.6ms** in Flow-Like.

| Metric | Flow-Like | Typical workflow engines |
|--------|-----------|------------------------|
| **Execution speed** | ~244,000 workflows/sec | ~200 workflows/sec |
| **Latency per workflow** | ~0.6ms | ~50-500ms |
| **Engine** | Rust (native compiled) | Python / Node.js (interpreted) |

That 1000x performance gap means real workflows can run on resource-constrained devices — phones, edge hardware, Raspberry Pis — not just beefy cloud servers. And on powerful machines, it means processing millions of executions without breaking a sweat.

### Full visibility into every execution

Most workflow tools show a green checkmark and move on. You're left guessing where data came from and why the result looks the way it does.

Flow-Like workflows are **fully typed** — they track *what data flows where* and *why*. Every input, transformation, and output is recorded with complete lineage and audit trails.

- **Data Origins** — See exactly where each value came from: the API response, the file, the user input.
- **Transformations** — Every validation, enrichment, and reformatting step is visible and traceable.
- **Clear Contracts** — Type-safe input/output definitions catch errors before deployment, not in production.
- **Three Perspectives** — Process view for business, Data view for analysts, Execution view for engineers. Same workflow, different lenses.

### And everything else

- **AI-Native** — Run LLMs locally or in the cloud with guardrails, approval gates, and full execution tracing on every call.
- **White-Label Ready** — Embed the editor in your product. Your logo, your colors, your brand. SSO, usage metering, and per-tenant scoping included.
- **Source Available** — BSL license, free for the vast majority of users (<2,000 employees and <$300M ARR).

---

## How it compares

| Feature | Flow-Like | n8n | Zapier / Make | Temporal |
|---------|-----------|-----|---------------|----------|
| Runs on your device | ✅ Desktop, phone, edge, server | ⚠️ Needs a server | ❌ Cloud only | ⚠️ Needs infrastructure |
| Works 100% offline | ✅ Full capability | ⚠️ Partial | ❌ Requires internet | ✅ Self-hosted |
| Type safety | ✅ Fully typed | ❌ Runtime only | ❌ None | ⚠️ Language-level |
| Data lineage / audit trail | ✅ Complete | ❌ Limited | ❌ None | ⚠️ Via logging |
| Performance | ✅ ~244K/sec (Rust) | ⚠️ ~200/sec (Node) | ⚠️ Cloud-limited | ⚠️ Go-based |
| Visual builder | ✅ Full IDE | ✅ Good | ✅ Simple | ❌ Code only |
| UI builder | ✅ Built-in | ❌ None | ❌ None | ❌ None |
| LLM orchestration | ✅ Built-in + guardrails | ⚠️ Via nodes | ⚠️ Via integrations | ❌ Manual |
| White-label / embed | ✅ Full customization | ❌ Branded | ❌ Branded | ❌ No UI |
| Business process views | ✅ Process / Data / Execution | ❌ Single view | ❌ Single view | ❌ Code only |
| License | Source Available (BSL) | Sustainable Use | Proprietary | MIT |

---

## Quick Start

| 💻 Desktop App | ☁️ Web App | 📱 Mobile App | ⚙️ From Source |
|:---:|:---:|:---:|:---:|
| **[Download Now](https://flow-like.com/download)** | **[Try Online](https://app.flow-like.com)** | **[Coming Soon](https://flow-like.com)** | **[Build Yourself](#build-from-source)** |
| macOS · Windows · Linux | Available now | iOS · Android | Latest features |

---

## The Ecosystem

### 🎨 Visual Workflow Studio

A no-code IDE for building workflows. Smart wiring with type-aware pins, inline execution feedback, live validation, and snapshot-based debugging.

<p align="center">
  <img src="assets/recording.gif" alt="Visual Studio" width="100%" />
</p>

### 🧩 900+ Built-in Nodes

APIs & webhooks, databases, file processing (Excel, CSV, PDF), AI models & computer vision, messaging (Slack, Discord, email), IoT, logic & control flow, security & auth — and growing.

**[→ Explore the Node Catalog](https://docs.flow-like.com/)**

### 🤖 AI-Powered Workflows

Download and run LLMs, vision models, and embeddings locally or in the cloud. Every AI decision is logged with full context — inputs, outputs, model version, and reasoning trace.

<p align="center">
  <img src="https://cdn.flow-like.com/website/SelectYourModel.webp" alt="AI Model Catalog" width="100%" />
</p>

### 📦 Apps & Templates

Package workflows as shareable applications with built-in storage. Run them offline or in the cloud. Browse the template store or share your own.

<p align="center">
  <img src="https://cdn.flow-like.com/website/CreateApp.webp" alt="Create Apps" width="48%" />
  <img src="https://cdn.flow-like.com/website/Store.webp" alt="Template Store" width="48%" />
</p>

---

## Who it's for

<table>
<tr>
<td width="33%" valign="top">

### 👨‍💻 Developers

- Runs on any device — laptop, phone, edge, server
- Type-safe data contracts
- Custom node SDK (Rust)
- Git-based version control
- Source available codebase

</td>
<td width="33%" valign="top">

### 📊 Business & Analysts

- No-code drag-and-drop builder
- Process, Data, and Technical views
- Approval workflows and change tracking
- Shareable apps and templates

</td>
<td width="33%" valign="top">

### 🏢 IT & Operations

- RBAC and enterprise governance
- Complete audit trails
- Air-gap and offline deployment
- SSO / OIDC integration
- Compliance-ready (GDPR, SOC2)

</td>
</tr>
</table>

---

## Build from Source

```bash
# Prerequisites: mise, Rust, Bun, Tauri prerequisites, Protobuf compiler
# Full guide: https://docs.flow-like.com/contributing/getting-started/

git clone https://github.com/TM9657/flow-like.git
cd flow-like
mise trust && mise install   # install toolchains (Rust, Bun, Node, Python, uv)
bun install                  # install Node packages
mise run build:desktop       # production build
```

All dev / build / deploy tasks are defined in the root [`mise.toml`](./mise.toml).
Run `mise tasks` to see every available task, or `mise run <task>` to execute one:

```bash
mise run dev:desktop:mac:arm   # dev mode – macOS Apple Silicon
mise run dev:web               # dev mode – Next.js web app
mise run build:desktop         # production desktop build
mise run fix                   # auto-fix lint (clippy + fmt + biome)
mise run check                 # run all checks without fixing
```

> 💡 Platform-specific hints for macOS, Windows, and Linux are in the [docs](https://docs.flow-like.com/).

---

## White-Label & Customization

Embed the visual editor in your application, or run the engine headlessly behind the scenes.

- **Themes** — Catppuccin, Cosmic Night, Neo-Brutalism, Soft Pop, Doom, or create your own
- **Design Tokens** — Map your brand palette with dark/light mode support
- **SSO** — OIDC/JWT with scoped secrets per tenant
- **Usage Metering** — Per-tenant quotas, event tracking, audit trails
- **SDKs & APIs** — Control workflows programmatically

### 🔌 SDKs

Integrate Flow-Like into your applications with official SDKs:

| Language | Package | Install |
|----------|---------|---------|
| **Node.js / TypeScript** | [`@flow-like/sdk`](https://www.npmjs.com/package/@flow-like/sdk) | `npm install @flow-like/sdk` |
| **Python** | [`flow-like`](https://pypi.org/project/flow-like/) | `uv add flow-like` |

Both SDKs support workflows, file management, LanceDB, chat completions, embeddings, and optional [LangChain](https://www.langchain.com/) integration. **[→ SDK Docs](https://docs.flow-like.com/dev/sdks/overview)**

Perfect for SaaS platforms, internal tools, client portals, and embedded automation.

---

## Contributing

We welcome contributions of all kinds — new nodes, bug fixes, docs, themes, and ideas.

**→ [Read CONTRIBUTING.md](./CONTRIBUTING.md)** for setup instructions and guidelines.

**→ [Browse `good first issue`](https://github.com/TM9657/flow-like/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)** to find a place to start.

**→ [Join Discord](https://discord.com/invite/mdBA9kMjFJ)** for questions and discussion.

---

<details>
<summary><strong>📸 Screenshots & Gallery</strong></summary>
<br/>

**Team & Access Management**

<p align="center">
  <img src="https://cdn.flow-like.com/website/TeamManagement.webp" alt="Team Management" width="100%" />
</p>

<p align="center">
  <img src="https://cdn.flow-like.com/website/RightsAndRoles.webp" alt="Rights and Roles" width="100%" />
</p>

**Built-in Storage & Search**

<p align="center">
  <img src="https://cdn.flow-like.com/website/Storage.webp" alt="Storage" width="100%" />
</p>

*Files, tables, and hybrid keyword+vector search — right on the canvas. No extra services needed.*

</details>

<details>
<summary><strong>❓ FAQ</strong></summary>
<br/>

**Is Flow-Like free to use?**
Most likely, yes. Flow-Like uses the Business Source License (BSL), which is free if your organization has fewer than 2,000 employees and less than $300M in annual recurring revenue. This covers startups, SMBs, and most enterprises. [Read the full license](./LICENSE).

**Can I run it completely offline?**
Yes, 100%. Flow-Like works fully offline on your local machine — ideal for air-gapped networks and secure environments. Switch to online mode anytime to collaborate.

**Can I embed it in my product?**
Yes. Flow-Like is white-label ready — embed the visual editor, customize the theme to your brand, integrate SSO, or run just the engine headlessly.

**What languages can I use?**
The visual builder is no-code. For custom nodes, you write Rust. SDKs and REST APIs are available for programmatic control.

**Is it production-ready?**
Flow-Like is actively developed and used in production. We recommend thorough testing for mission-critical workflows. See the [releases page](https://github.com/TM9657/flow-like/releases) for version stability.

**How do I get support?**
[Discord](https://discord.com/invite/mdBA9kMjFJ) for quick help, [Docs](https://docs.flow-like.com) for guides, or [GitHub Issues](https://github.com/TM9657/flow-like/issues) for bugs and features.

</details>

<details>
<summary><strong>🏗️ Built With</strong></summary>
<br/>

Flow-Like stands on the shoulders of incredible open-source projects:

**Frontend:** [React Flow](https://github.com/xyflow/xyflow) · [Radix UI](https://github.com/radix-ui/primitives) · [shadcn/ui](https://github.com/shadcn-ui/ui) · [Next.js](https://github.com/vercel/next.js) · [Tailwind CSS](https://github.com/tailwindlabs/tailwindcss) · [Framer Motion](https://github.com/framer/motion)

**Desktop & Runtime:** [Tauri](https://github.com/tauri-apps/tauri) · [Rust](https://github.com/rust-lang/rust) · [Tokio](https://github.com/tokio-rs/tokio) · [Axum](https://github.com/tokio-rs/axum)

**AI & ML:** [llama.cpp](https://github.com/ggerganov/llama.cpp) · [Candle](https://github.com/huggingface/candle) · [ONNX Runtime](https://github.com/microsoft/onnxruntime)

**Data:** [Zustand](https://github.com/pmndrs/zustand) · [TanStack Query](https://github.com/TanStack/query) · [Dexie.js](https://github.com/dexie/Dexie.js) · [SeaORM](https://github.com/SeaQL/sea-orm) · [Zod](https://github.com/colinhacks/zod)

**Tooling:** [mise](https://github.com/jdx/mise) · [Bun](https://github.com/oven-sh/bun) · [Vite](https://github.com/vitejs/vite) · [Biome](https://github.com/biomejs/biome)

Thank you to all maintainers and contributors of these projects! 🙏

</details>

---

## 📊 Project Stats

<p align="center">
  <picture>
    <img src="https://repobeats.axiom.co/api/embed/6fe5df31b9a96f584f8898beb4457bd8aa3852f1.svg" alt="Repobeats analytics" width="48%" />
  </picture>
  <picture>
    <img src="https://api.star-history.com/svg?repos=TM9657/flow-like&type=Date" alt="Star History" width="48%" />
  </picture>
</p>

---

<p align="center">
  <a href="https://flow-like.com">Website</a> ·
  <a href="https://docs.flow-like.com">Docs</a> ·
  <a href="https://flow-like.com/download">Download</a> ·
  <a href="https://flow-like.com/blog">Blog</a>
</p>

<p align="center">
  <strong>Made with ❤️ in Munich, Germany</strong><br/>
  <sub>
    <a href="./LICENSE">License</a> ·
    <a href="./CODE_OF_CONDUCT.md">Code of Conduct</a> ·
    <a href="./SECURITY.md">Security</a>
  </sub>
</p>
