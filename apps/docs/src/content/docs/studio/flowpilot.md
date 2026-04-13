---
title: FlowPilot AI Assistant
description: Using AI to build and edit your Flows with FlowPilot
sidebar:
  order: 15
---

**FlowPilot** is Flow-Like's integrated AI assistant that helps you build, edit, and understand your workflow automations using natural language. It supports multiple AI providers and specialized agents for different tasks.

## Overview

FlowPilot can:
- **Create nodes and connections** from natural language descriptions
- **Explain existing flows** and help you understand complex automations
- **Debug issues** by analyzing execution logs and suggesting fixes
- **Search the node catalog** to find the right nodes for your task
- **Generate complete workflows** from high-level requirements

## AI Providers

FlowPilot supports two AI provider modes:

### Bits (Default)
Uses your configured model bits from your user profile. This is the default mode and works with any LLM provider you've set up in Flow-Like.

### GitHub Copilot
Uses the GitHub Copilot SDK directly for AI-powered assistance in the desktop app. This mode provides access to GitHub's latest AI models and uses your local Copilot CLI session.

## Setting Up GitHub Copilot

To use FlowPilot with GitHub Copilot, you need to install and configure the GitHub Copilot CLI.

### Prerequisites

- An active [GitHub Copilot subscription](https://github.com/features/copilot/plans)
- One of the supported platforms:
  - macOS
  - Linux
  - Windows (PowerShell v6 or higher)

:::note
If you have access to GitHub Copilot via your organization or enterprise, ensure your administrator has enabled GitHub Copilot CLI in the organization settings. See [Managing policies for Copilot](https://docs.github.com/copilot/managing-copilot/managing-github-copilot-in-your-organization/managing-github-copilot-features-in-your-organization/managing-policies-for-copilot-in-your-organization) for more information.
:::

### Installation

Choose your preferred installation method:

#### macOS and Linux (Homebrew)
```bash
brew install copilot-cli
```

For the prerelease version:
```bash
brew install copilot-cli@prerelease
```

#### Windows (WinGet)
```powershell
winget install GitHub.Copilot
```

For the prerelease version:
```powershell
winget install GitHub.Copilot.Prerelease
```

#### npm (All Platforms)
```bash
npm install -g @github/copilot
```

For the prerelease version:
```bash
npm install -g @github/copilot@prerelease
```

#### Install Script (macOS and Linux)
```bash
curl -fsSL https://gh.io/copilot-install | bash
```

Or with wget:
```bash
wget -qO- https://gh.io/copilot-install | bash
```

:::tip
Use `| sudo bash` to run as root and install to `/usr/local/bin`. Set `PREFIX` to install to a custom directory.
:::

### Authentication

After installation, you need to authenticate with your GitHub account:

1. **Launch the CLI** by running `copilot` in your terminal
2. **Follow the login prompt** - enter the `/login` command if prompted
3. **Complete the authentication** in your browser

#### Alternative: Personal Access Token (PAT)

You can also authenticate using a fine-grained PAT:

1. Visit [GitHub Token Settings](https://github.com/settings/personal-access-tokens/new)
2. Under "Permissions," add the "Copilot Requests" permission
3. Generate your token
4. Set the `GH_TOKEN` or `GITHUB_TOKEN` environment variable with your token

### Verifying Installation

Run `copilot` in your terminal. You should see the Copilot CLI interface with authentication status.

## Using FlowPilot

### Switching Providers

In the FlowPilot panel, you'll see provider toggle buttons:
- **Bits** - Use your configured model bits
- **Copilot** - Use GitHub Copilot

Click on the Copilot button to switch. In the desktop app, FlowPilot connects to the local Copilot CLI. In browser-based FlowPilot sessions, the Copilot option stays disabled and Bits remains the available provider.

### Selecting Models

When using GitHub Copilot, you can choose from available models:
- **Claude Sonnet 4.5** (default)
- **Claude Sonnet 4**
- **GPT-5**
- And other available models

Use the model selector dropdown to switch between models.

### Chat Interface

The FlowPilot chat interface allows you to:
1. **Ask questions** about your current flow
2. **Request modifications** to nodes and connections
3. **Get explanations** of what specific nodes do
4. **Debug errors** by sharing log output

Example prompts:
- "Add an HTTP request node that calls the OpenAI API"
- "Connect the output of the JSON parser to the email sender"
- "Explain what this flow does"
- "Why is this node failing? Here's the error..."

### Specialized Agents

FlowPilot includes specialized agents optimized for different tasks:

#### Frontend Agent
Focused on UI/UX development:
- Creating responsive UI components
- React patterns and hooks
- CSS/Tailwind styling
- A2UI component system

#### Backend Agent
Focused on workflow and data processing:
- Flow graph design and node connections
- Data transformation and processing
- API integrations
- Error handling and retry logic

#### General Agent
Can handle both frontend and backend tasks, automatically determining the best approach based on your request.

## How It Works

When you send a message to FlowPilot, it:

1. **Analyzes your request** and the current board context
2. **Searches the node catalog** to find relevant nodes
3. **Plans the changes** needed to fulfill your request
4. **Generates commands** to modify the board
5. **Applies changes** while maintaining undo/redo history

All changes are applied through the same command system used by manual editing, so you can always undo AI-generated changes.

## Rolling Context Budget

FlowPilot compacts older conversation turns before each request instead of sending the full transcript every time. It keeps recent turns verbatim, summarizes accepted work from older turns, and sends current-turn images separately from the text prompt.

This keeps long workflow sessions usable without duplicating the latest request or letting the prompt grow without bound.

## Privacy and Security

- **Local processing**: The Copilot CLI runs locally on your machine
- **GitHub authentication**: Uses your existing GitHub credentials
- **No data storage**: Conversations are not stored on GitHub's servers beyond what's needed for processing
- **Explicit approval**: All changes require your approval before being applied

## Troubleshooting

### Copilot CLI not found
Ensure the Copilot CLI is installed and in your PATH. Try running `copilot` directly in your terminal.

### Authentication issues
Run `copilot` in your terminal and use the `/login` command to re-authenticate.

### Copilot option is disabled
GitHub Copilot in FlowPilot currently runs only in the desktop app. Use the Bits provider in web sessions.

### Model not available
Some models may require specific Copilot subscription tiers. Check your subscription level if a model is unavailable.

## Resources

- [GitHub Copilot CLI Documentation](https://docs.github.com/copilot/concepts/agents/about-copilot-cli)
- [GitHub Copilot CLI Repository](https://github.com/github/copilot-cli)
- [Copilot Plans](https://github.com/features/copilot/plans)
