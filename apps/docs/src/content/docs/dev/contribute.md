---
title: Contribute
description: How to Contribute to Flow-Like
sidebar:
  order: 40
---

We welcome contributions to Flow-Like! Whether you're fixing bugs, adding features, improving documentation, or writing new nodes — your help makes Flow-Like better for everyone.

## Quick Start

1. **Fork and clone** the repository:

   ```bash
   git clone https://github.com/your-username/flow-like.git
   cd flow-like
   ```

2. **Create a feature branch**:

   ```bash
   git checkout -b feature/your-feature-name
   ```

3. **Make changes**, commit, and push:

   ```bash
   git commit -m "Describe your change here"
   git push origin feature/your-feature-name
   ```

4. **Open a Pull Request** on the main repository.

## Development Setup

Before contributing, set up your development environment:

1. Install prerequisites ([Building from Source](/dev/build/))
2. Run the development server:

   ```bash
   bun install
   mise run dev:desktop:mac:arm   # or your platform variant
   ```

3. Format and lint before committing:

   ```bash
   mise run fix    # runs cargo clippy --fix, cargo fmt, and bunx biome check --write
   ```

## Contribution Areas

### Core Features

- Workflow execution engine (`packages/core`)
- API and multi-tenancy (`packages/api`)
- Storage backends (`packages/storage`)

### Node Catalog

Adding new nodes is one of the best ways to contribute:

- Standard library nodes (`packages/catalog/std`)
- AI/ML nodes (`packages/catalog/llm`, `packages/catalog/ml`)
- Data processing (`packages/catalog/data`, `packages/catalog/processing`)

→ See [Writing Nodes](/dev/writing-nodes/) for a complete guide.

### Documentation

- Improve existing docs in `apps/docs/src/content/docs/`
- Add tutorials and examples
- Translate the website (`apps/website/src/i18n/`)

→ See [Translations](/dev/translations/) for localization help.

### Frontend

- Desktop app (`apps/desktop`)
- Web app (`apps/web-app`)
- Shared UI components (`packages/ui`)

## Code Guidelines

### Rust

- Use `cargo fmt` for formatting
- Run `cargo clippy` and address warnings
- Follow existing patterns in the codebase
- Add tests for new functionality
- Use `anyhow` for error handling in applications
- Prefer early returns for readability

### TypeScript/React

- Use functional components with hooks
- Follow the patterns in existing components
- Use Tailwind CSS for styling
- Import shadcn components from `packages/ui`

## Pull Request Checklist

Before submitting a PR:

- [ ] Code compiles without errors (`cargo check`)
- [ ] Tests pass (`cargo test`)
- [ ] Code is formatted (`cargo fmt`, `bunx biome check`)
- [ ] Clippy warnings addressed (`cargo clippy`)
- [ ] Documentation updated if needed
- [ ] Commit messages are clear and descriptive

## Good First Issues

New to the project? Look for issues labeled [`good first issue`](https://github.com/TM9657/flow-like/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22).

## Reporting Issues

When opening an issue:

- Use a clear, descriptive title
- Include steps to reproduce (for bugs)
- Provide environment details (OS, versions)
- Include relevant logs or screenshots

## Security Issues

For security vulnerabilities, **do not open a public issue**. Report privately to [security@great-co.de](mailto:security@great-co.de).

## Code of Conduct

By contributing, you agree to our [Code of Conduct](https://github.com/TM9657/flow-like/blob/main/CODE_OF_CONDUCT.md). We expect respectful, constructive interactions.

## Thank You

Your contributions make Flow-Like better. We appreciate every bug fix, feature, and documentation improvement!
