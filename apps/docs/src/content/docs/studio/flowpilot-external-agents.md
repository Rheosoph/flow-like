---
title: External coding agents in FlowPilot
description: Install, sign in to, verify, and troubleshoot the GitHub Copilot, Claude Code, and Codex backends used by FlowPilot.
sidebar:
  order: 16
---

FlowPilot can use an existing **GitHub Copilot**, **Claude Code**, or **Codex** subscription through the provider's command-line app (CLI). These integrations are currently available in the **Flow-Like desktop app**.

You only have to install and sign in to the CLI. **Do not add FlowPilot as a permanent MCP server.** Flow-Like configures the selected CLI and its FlowPilot tools automatically.

## What gets installed

The parts run like this:

1. Flow-Like starts the selected CLI on your computer.
2. Flow-Like opens a local tool session for that request. Claude Code and Codex use a temporary MCP server; GitHub Copilot uses its local SDK session.
3. The CLI can see only the FlowPilot tools needed for the task.
4. Flow-Like closes the local session when the request finishes.

Your normal provider login is reused. Flow-Like does not ask you to copy an API key into the app.

:::note
External coding agents require the desktop app. In the web app, use a model configured in your Flow-Like profile (the **Profile** or **Bits** option).
:::

## Set up GitHub Copilot

### 1. Install the CLI

On macOS or Linux, use Homebrew:

```bash
brew install copilot-cli
```

You can also use GitHub's install script on macOS or Linux:

```bash
curl -fsSL https://gh.io/copilot-install | bash
```

On Windows, use WinGet:

```powershell
winget install GitHub.Copilot
```

GitHub also supports npm on every platform with Node.js 22 or newer. See the [official GitHub Copilot CLI installation guide](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli) for all options.

### 2. Verify the installation

Open a new terminal and run:

```bash
copilot --version
```

### 3. Sign in

Start Copilot, then enter `/login` when prompted:

```bash
copilot
```

You can also run `copilot login`. GitHub Copilot CLI requires an active Copilot plan; organization-managed plans must allow the Copilot CLI policy.

### 4. Connect from Flow-Like

1. Fully quit and reopen Flow-Like after installing the CLI.
2. Open **FlowPilot**.
3. Open the provider/model picker and select **GitHub Copilot**.
4. Select one of the models returned by Copilot.
5. Send: `Explain what you can access in Flow-Like. Do not change anything.`

If the answer appears without a connection error, the CLI, authentication, and FlowPilot tool connection are working.

## Set up Claude Code

### 1. Install the CLI

On macOS, Linux, or WSL, use Anthropic's recommended native installer:

```bash
curl -fsSL https://claude.ai/install.sh | bash
```

On macOS, you can alternatively use Homebrew:

```bash
brew install --cask claude-code
```

For Windows and other supported installation methods, follow the [official Claude Code setup guide](https://code.claude.com/docs/en/getting-started).

### 2. Verify the installation

Open a new terminal and run:

```bash
claude --version
claude doctor
```

Both commands should finish without an installation error.

### 3. Sign in

```bash
claude auth login
```

Complete the browser sign-in, then verify it:

```bash
claude auth status --text
```

Claude Code requires an eligible Claude subscription, Anthropic Console account, or a supported enterprise provider. The free Claude.ai plan does not include Claude Code.

### 4. Connect from Flow-Like

1. Fully quit and reopen Flow-Like after installing the CLI.
2. Open **FlowPilot**.
3. Open the provider/model picker and select **Claude Code**.
4. Select the configured default model or another model returned by Claude Code.
5. Send: `Explain what you can access in Flow-Like. Do not change anything.`

If the answer appears without a connection error, the CLI, authentication, and temporary MCP connection are working.

## Set up Codex

### 1. Install the CLI

On macOS or Linux, use OpenAI's standalone installer:

```bash
curl -fsSL https://chatgpt.com/codex/install.sh | sh
```

For Windows, npm, Homebrew, and update instructions, follow the [official Codex CLI guide](https://developers.openai.com/codex/cli/).

### 2. Verify the installation

Open a new terminal and run:

```bash
codex --version
```

### 3. Sign in

```bash
codex login
```

Complete the browser sign-in, then verify it:

```bash
codex login status
```

Codex supports signing in with ChatGPT or with an OpenAI API key. See [Codex authentication](https://developers.openai.com/codex/auth/) for the differences.

### 4. Connect from Flow-Like

1. Fully quit and reopen Flow-Like after installing the CLI.
2. Open **FlowPilot**.
3. Open the provider/model picker and select **Codex**.
4. Select the configured default model or another model returned by Codex.
5. Send: `Explain what you can access in Flow-Like. Do not change anything.`

If the answer appears without a connection error, the CLI, authentication, and temporary MCP connection are working.

## Troubleshooting

### Flow-Like cannot find the CLI

First confirm the relevant command works in a **new** terminal:

```bash
copilot --version
# or
claude --version
# or
codex --version
```

Then fully quit and reopen Flow-Like. Apps opened from the Dock or Finder can have a different command search path than your terminal, so restarting after installation matters.

Flow-Like checks common native, Homebrew, npm, local-user, and supported editor-extension locations. If you use a custom location, start Flow-Like with one of these environment variables set to the full executable path:

```bash
COPILOT_CLI_PATH=/absolute/path/to/copilot
CLAUDE_CODE_CLI_PATH=/absolute/path/to/claude
CODEX_CLI_PATH=/absolute/path/to/codex
```

Do not point the variable at a shell alias. It must resolve to the executable or the directory that contains it.

### The CLI is installed but FlowPilot says it is not authenticated

Check or refresh the provider login in a terminal:

```bash
copilot login
# or
claude auth status --text
# or
codex login status
```

If it is signed out, run `copilot login`, `claude auth login`, or `codex login`, finish the browser flow, and reopen Flow-Like.

### macOS asks for access to Desktop ("Schreibtisch")

FlowPilot does **not** need your Desktop folder to exchange tool messages, and its CLI lookup does not search Desktop. External agents run from a neutral temporary directory. You can choose **Don't Allow**. macOS separately protects Desktop, Documents, and Downloads, so it shows this prompt whenever an app or child process tries to inspect one of those locations.

If the prompt appears immediately after selecting an external provider, note what you clicked, choose **Don't Allow**, and report it through [Flow-Like support](/start/support/). Include your macOS version, Flow-Like version, selected provider, and whether Flow-Like was started from Applications, the Dock, a terminal, or a downloaded disk image.

You can review this permission later under **System Settings → Privacy & Security → Files & Folders**. Do not grant Full Disk Access just to make FlowPilot work.

### The provider connects, but FlowPilot tools are unavailable

The temporary FlowPilot tool connection exists only while FlowPilot is handling a request, so it will not appear as a permanent entry in your provider's configuration or in an MCP inspector opened later.

Try a new read-only test message in FlowPilot. If it fails, copy the complete connection error and restart Flow-Like. You should not need to edit `~/.claude.json`, `.mcp.json`, or `~/.codex/config.toml` for this integration.

### A company-managed account does not work

Ask your administrator whether the selected CLI is enabled for your account. Authentication can succeed while an organization policy still blocks a model, local client, MCP server, or tool.

## Official references

- [GitHub Copilot CLI installation](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli)
- [GitHub Copilot CLI configuration](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/configure-copilot-cli)
- [Claude Code installation and authentication](https://code.claude.com/docs/en/getting-started)
- [Claude Code CLI reference](https://code.claude.com/docs/en/cli-usage)
- [Claude Code MCP support](https://code.claude.com/docs/en/mcp)
- [Codex CLI](https://developers.openai.com/codex/cli/)
- [Codex authentication](https://developers.openai.com/codex/auth/)
- [Codex MCP support](https://developers.openai.com/codex/mcp/)
- [Apple: control access to files and folders on Mac](https://support.apple.com/guide/mac-help/control-access-to-files-and-folders-on-mac-mchld5a35146/mac)
