---
title: AI Models & Setup
description: Configure model providers and select models for Flow-Like GenAI workflows
sidebar:
  order: 2
---

Flow-Like model nodes use the providers and models available in the active profile. Configure credentials and endpoints once, then either select a provider model explicitly or let **Find Model** choose from the available catalog using preferences.

## Configure the active profile

Use the profile and model settings to:

1. add a provider connection;
2. enter the required credential or local endpoint;
3. discover or enable the models you intend to use;
4. test the connection;
5. save the profile;
6. run a small workflow with the selected model.

See [Profiles](/start/profiles/) and [AI models in the getting-started guide](/start/models/) for the current interface.

Store provider credentials in the profile or secret-backed configuration. Do not put API keys into boards, prompts, logs, or documentation screenshots.

## Provider nodes

The generated provider catalog currently includes model builders for:

| Provider family | Examples |
|-----------------|----------|
| Major hosted APIs | OpenAI, Anthropic, Gemini, Vertex AI, AWS Bedrock |
| Hosted inference and routing | Groq, OpenRouter, Together AI, Perplexity, Huggingface |
| Other hosted providers | Cohere, Deepseek, Mistral, Moonshot AI, xAI, Hyperbolic, VoyageAI |
| Local or compatible endpoints | Ollama, LM Studio, Mozilla any-llm |
| Personal agent accounts | Claude Code, Codex (ChatGPT), GitHub Copilot, Microsoft 365 Copilot |
| Additional catalog providers | Galadriel, Mira |

Browse [Generative model provider nodes](/nodes/ai/generative/provider/) for the current set and each node's inputs. Provider availability and model lists can change independently of the docs.

## Add models from an agent account

In the model catalog, choose **Add model**, select the provider, and enter a model ID and its credentials. Save the model and activate it in the profile you run workflows with. These entries are ordinary model Bits, so they connect to **Invoke Model** and **Agent from Model** through the existing Model pin. Provider nodes can also build a Bit directly and report whether its provider is available.

| Provider | Setup and execution |
|----------|---------------------|
| Claude Code | Desktop only. Install and sign in to the Claude CLI, then select a discovered model or enter its ID. Flow-Like launches the CLI for each completion; an open terminal is unnecessary. |
| Codex (ChatGPT) | Supply a ChatGPT access token, or use the desktop's cached Codex `auth.json` credentials. Local credential lookup does not refresh tokens or read OS keychain storage. |
| GitHub Copilot | Supply a GitHub token authorized for Copilot, or an already exchanged Copilot API token. This model provider does not reuse FlowPilot's Copilot SDK session. |
| Microsoft 365 Copilot | Supply a delegated Microsoft Graph token with Copilot Chat access. The provider selects its own underlying model. This adapter supports text conversations and streaming, without caller-defined tools or model sampling controls. |

Explicit-token providers can execute through the browser's server backend. Claude Code and local Codex credential lookup require desktop execution. Server runs never use the server operator's CLI login.

Claude Code's adapter sends the full Flow-Like conversation in a fresh CLI prompt and returns a validated assistant turn, including requested Flow-Like tool calls. Flow-Like executes those tools. Its stream output arrives after the full turn has been validated. Codex and GitHub Copilot use Rig's model providers and retain Flow-Like's existing agent loop. Provider controls differ: the Claude Code adapter does not apply model sampling settings, and the Codex subscription backend does not honor temperature or maximum output tokens.

**Find Model** skips external providers that fail the execution host's readiness check. A saved use-case selection also falls back to another active profile model when its external provider is unavailable. Missing credentials, expired tokens, unavailable CLI authentication, and unsupported local execution can trigger fallback. Readiness does not guarantee remaining quota, model entitlement, or a successful later network request. Errors after generation starts are returned to the workflow; the runtime does not replay tool actions on another model.

## Explicit model selection

Use a provider-specific model node when the board requires a known provider configuration. Examples include:

- [OpenAI Model](/nodes/ai/generative/provider/ai-generative-build-openai/)
- [Anthropic Model](/nodes/ai/generative/provider/ai-generative-build-anthropic/)
- [Gemini Model](/nodes/ai/generative/provider/ai-generative-build-gemini/)
- [AWS Bedrock Model](/nodes/ai/generative/provider/ai-generative-build-bedrock/)
- [Ollama Model](/nodes/ai/generative/provider/ai-generative-build-ollama/)
- [LM Studio Model](/nodes/ai/generative/provider/ai-generative-build-lmstudio/)

Explicit selection is useful when:

- a workflow has been evaluated against one model configuration;
- data residency or provider policy is fixed;
- a provider-specific option is required;
- exact cost and behavior need controlled rollout.

Keep the model identifier configurable rather than scattering it across several boards.

## Preference-based selection

[Find Model](/nodes/ai/generative/ai-generative-find-model/) selects a model from the active profile using a `BitModelPreference`.

Build the preference with:

| Node | Purpose |
|------|---------|
| [Make Preferences](/nodes/ai/generative/preferences/ai-generative-make-preferences/) | Start a preference value and require multimodal capability when needed |
| [Set Preference Weight](/nodes/ai/generative/preferences/ai-generative-set-preference-weight/) | Weight cost, speed, reasoning, creativity, factuality, function calling, safety, openness, multilinguality, or coding |
| [Set Model Hint](/nodes/ai/generative/preferences/ai-generative-set-model-hint/) | Add a soft hint for a desired model family |

Preference weights guide selection; they are not hard guarantees of quality. Evaluate the selected-model behavior for the workflow and log the actual model used with each run.

Use preference-based selection when the board can tolerate a compatible alternative and the active profile may differ across environments.

## Match capability to the task

| Task | Required capability to verify |
|------|-------------------------------|
| Chat or generation | Text generation and sufficient context |
| Tool-using agent | Reliable function or tool calling |
| Structured extraction | Required tool call and JSON Schema adherence |
| Image understanding | Multimodal or vision input |
| RAG indexing | Embedding model with stable vector dimension |
| Speech | Matching speech-to-text or text-to-speech model type |
| Image or video generation | Corresponding generation model and options |

A provider may expose several model types. A text-generation model is not automatically an embedding, speech, image, or video model.

## Local models

[Ollama Model](/nodes/ai/generative/provider/ai-generative-build-ollama/) and [LM Studio Model](/nodes/ai/generative/provider/ai-generative-build-lmstudio/) connect to compatible local services.

Before using a local model:

- confirm the service is reachable from the execution backend;
- verify model type and tool or vision support;
- measure memory, accelerator, and disk requirements on the target machine;
- test concurrency and timeout behavior;
- define what should happen when the local service is unavailable.

Hardware requirements depend on model architecture, quantization, context size, and runtime. Use the model and runtime documentation instead of a universal RAM estimate.

## Embedding models

RAG requires an embedding model for documents and queries. Use [Load Embedding Model](/nodes/ai/embedding/load-model/), [Embed Document](/nodes/ai/embedding/embed-document/), and [Embed Query](/nodes/ai/embedding/embed-query/).

Index and query with the same embedding model and configuration. Changing the model normally requires rebuilding the vector index.

## Model configuration

History nodes can set options such as maximum tokens, temperature, top-p, response format, streaming, seed, and stop words. Provider support differs.

Choose settings through evaluation:

- lower variability for extraction and governed answers;
- enough output budget for the response contract;
- streaming only when partial output can be handled safely;
- response format compatible with the downstream parser;
- explicit timeout and retry behavior.

Do not assume a provider interprets every sampling parameter identically.

## Evaluate before rollout

Maintain task-specific cases and compare:

- correctness and completeness;
- tool or schema adherence;
- refusal and uncertainty behavior;
- latency and timeout rate;
- token usage and cost;
- multilingual and domain behavior where relevant;
- safety on adversarial or sensitive inputs.

Record the provider, model identifier, profile or configuration version, prompt version, and relevant settings with evaluation results.

## Troubleshooting

| Symptom | Check |
|---------|-------|
| No models available | Active profile, provider connection, model discovery, network reach |
| Authentication fails | Credential scope, expiration, endpoint, secret handling |
| Local model is unreachable | Service address from the execution backend, firewall, process state |
| Tool calls fail | Model capability, tool schema, iteration and timeout limits |
| Structured extraction fails | Function-call support, schema validity, selected model |
| Output changes between environments | Active profile, selected model, preference result, settings |
| Context errors | Input size, history length, retrieval count, output budget |

## Next steps

- [Chat and conversations](/topics/genai/chat/)
- [RAG and knowledge bases](/topics/genai/rag/)
- [AI agents](/topics/genai/agents/)
- [Extraction and structured output](/topics/genai/extraction/)
