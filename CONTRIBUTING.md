# Contributing to Flow-Like

Thank you for your interest in contributing to **Flow-Like**! Whether you're fixing a bug, writing a new node, improving docs, or just asking a good question — you're helping build a better workflow engine.

## 🚀 Quick Start

```bash
# 1. Fork & clone
git clone https://github.com/your-username/flow-like.git
cd flow-like

# 2. Install prerequisites
# - mise: https://mise.jdx.dev/getting-started.html
# - Tauri prerequisites: https://tauri.app/start/prerequisites/
# - Protobuf compiler: https://protobuf.dev/installation/

# 3. Install toolchains & dependencies
mise trust && mise install   # installs Rust, Bun, Node, Python, uv
bun install

# 4. Run in dev mode
mise run dev:desktop:mac:arm     # macOS Apple Silicon
mise run dev:desktop:mac:intel   # macOS Intel
mise run dev:desktop:win:x64     # Windows x64
mise run dev:desktop:linux:x64   # Linux x64
```

> **Full setup guide →** [docs.flow-like.com/contributing/getting-started](https://docs.flow-like.com/contributing/getting-started/)

---

## 🗂 Project Structure

Flow-Like is a Rust + TypeScript monorepo. Here's the lay of the land:

```
flow-like/
├── apps/
│   ├── desktop/          # Tauri desktop app (TypeScript + React)
│   └── web/              # Web app (Next.js)
├── packages/
│   ├── flow-like/        # Core Rust engine (DAG scheduler, execution runtime)
│   ├── flow-like-types/  # Shared type definitions
│   ├── flow-like-nodes/  # Built-in node implementations ← easiest place to contribute
│   ├── flow-like-ui/     # Shared React components
│   └── ...
├── tools/                # Build tooling & scripts
└── tests/                # Integration tests
```

> **Not sure where to start?** The `packages/flow-like-nodes/` crate is the easiest entry point — each node is a self-contained unit with clear input/output types.

---

## 🎯 Where to Contribute

| Area | Difficulty | Description |
|------|-----------|-------------|
| **New Nodes** | 🟢 Easy | Add integrations, data transforms, or utility nodes |
| **Bug Fixes** | 🟢–🟡 | Fix reported issues — check the [issue tracker](https://github.com/TM9657/flow-like/issues) |
| **Documentation** | 🟢 Easy | Tutorials, guides, API docs, README improvements |
| **UI/UX** | 🟡 Medium | Improve the visual editor, add themes, polish interactions |
| **Core Engine** | 🔴 Advanced | DAG scheduler, execution runtime, type system |
| **Testing** | 🟢–🟡 | Add test coverage for existing features |

**→ [Browse `good first issue` labels](https://github.com/TM9657/flow-like/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)**

**→ [Browse `help wanted` labels](https://github.com/TM9657/flow-like/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22)**

---

## 🔧 Development Workflow

### 1. Create a branch

```bash
git checkout -b feature/your-feature-name   # features
git checkout -b fix/issue-description        # bug fixes
```

### 2. Make your changes

**Auto-fix everything:**
```bash
mise run fix    # runs cargo clippy --fix, cargo fmt, and bunx biome check --write
```

**Rust code:**
- Run `cargo clippy` before committing — warnings should be resolved
- Run `cargo test` to ensure nothing breaks
- Follow existing code style and naming conventions

**TypeScript code:**
- Run `bunx biome check .` for linting and formatting
- Follow existing component patterns in `packages/flow-like-ui/`

### 3. Commit with a clear message

```bash
git commit -m "feat: add Discord webhook node"
git commit -m "fix: resolve DAG cycle detection edge case"
git commit -m "docs: add tutorial for creating custom nodes"
```

We loosely follow [Conventional Commits](https://www.conventionalcommits.org/) — prefixes like `feat:`, `fix:`, `docs:`, `refactor:`, `test:` help keep the changelog readable.

### 4. Push & open a PR

```bash
git push origin feature/your-feature-name
```

Then open a Pull Request against the `dev` branch. In your PR description:
- Describe **what** changed and **why**
- Link related issues (e.g., `Closes #42`)
- Include screenshots or GIFs for UI changes

---

## 📐 Code Guidelines

### Rust

- Write clear, idiomatic Rust — prefer `Result` over panics
- Add doc comments (`///`) to public types and functions
- Include tests for new features or bug fixes
- Keep dependencies minimal — check if existing crates already cover your need

### TypeScript / React

- Use TypeScript strictly — avoid `any` unless absolutely necessary
- Follow the existing component patterns (shadcn/ui + Tailwind)
- Keep components small and composable

### General

- Don't introduce new linters or formatters — we use Clippy (Rust) and Biome (TS)
- If a change touches public APIs, update the relevant documentation
- If you're unsure about an approach, open a [Discussion](https://github.com/TM9657/flow-like/discussions) first

---

## 🐛 Reporting Bugs

Open an issue with:

- **Clear title** describing the problem
- **Steps to reproduce** — the more specific, the better
- **Expected vs actual behavior**
- **Environment** — OS, app version (from Settings), desktop or web
- **Screenshots or screen recordings** if it's a visual issue

---

## 💡 Suggesting Features

We love feature ideas! Before opening an issue:

1. Search [existing issues](https://github.com/TM9657/flow-like/issues) and [Discussions](https://github.com/TM9657/flow-like/discussions) to avoid duplicates
2. Describe the **problem** you're trying to solve (not just the solution)
3. Include mockups or examples if possible

---

## 🔐 Security Issues

For security vulnerabilities, please **do not open a public issue**. Report privately to [security@great-co.de](mailto:security@great-co.de). See [SECURITY.md](./SECURITY.md) for details.

---

## 🤝 Code of Conduct

By participating, you agree to our [Code of Conduct](./CODE_OF_CONDUCT.md). Be respectful, constructive, and welcoming.

---

## 💬 Getting Help

Stuck? Have questions?

- **[Discord](https://discord.com/invite/mdBA9kMjFJ)** — fastest way to get help
- **[GitHub Discussions](https://github.com/TM9657/flow-like/discussions)** — longer-form questions and ideas
- **[Documentation](https://docs.flow-like.com)** — guides and API reference

---

## 🙌 Thank You

Every contribution matters — from a typo fix to a new node to a thoughtful bug report. Flow-Like is better because of contributors like you.