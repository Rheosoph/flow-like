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
| Classification, scoring, or yes/no assessments | SystemOne decision model |
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

For media or mixed content, connect the loaded model to **Embedding Model Info** to inspect its supported inputs, tasks, output dimensions, and joint combinations. **Embed Content** produces one vector from an item's ordered content parts. **Embed Content Batch** produces one vector per item and preserves item order. Text and an image inside one item are encoded together only when the model supports that combination. CLIP supports separate text and image inputs in a shared vector space.

Local embedding models run through ONNX. Existing text and image Bits keep their query prefixes, document prefixes, chunking, and pooling behavior. Versioned embedding Bits can select a sentence-transformer, CLIP, or EmbeddingGemma 2 adapter and assign their model files to named roles. EmbeddingGemma 2 supports text, images, audio, video, and joint inputs when its corresponding encoders are installed. Its available vector dimensions are 128, 256, 512, and 768. This adapter requires a host with local ML execution.

EmbeddingGemma 2 can also use an Internal remote fallback with `remote.model_id` set to `embeddinggemma-2`. A capable local host keeps executing the local model. Hosts without local execution use the authenticated API proxy, which supports all four modalities and joint inputs at 768 dimensions with an 8,192-token shared context. The remote pipeline has its own embedding-space identity; do not mix its vectors with a local index without verifying compatibility.

To load EmbeddingGemma 2, use the [pinned example Bit](https://github.com/Rheosoph/flow-like/blob/main/packages/model-provider/tests/fixtures/gemma2/bit.json) as the **Model Bit** input of **Load Embedding Model**. It declares the backbone, image encoder, audio encoder, tokenizer, and configuration files with download URLs and content hashes. Loading it installs about 505 MB of assets. The example works independently of the published model catalog.

Use **Embed Audio** or **Embed Video** to embed a `FlowPath` directly. Connect the file to **Source** and the loaded model to **Model**. Both nodes select the input modality from the file extension, regardless of the node's name: images use image embedding, audio uses audio embedding, video uses video embedding, and Markdown or other supported UTF-8 text files use text embedding. Extension matching ignores case. An unsupported extension, incompatible model, or corrupt file returns an error; the nodes do not guess another modality after decoding fails.

Both nodes expose **Purpose** and **Options**, including supported output dimensions, and return **Vector** plus **Result** with space and model metadata. **Embed Video** also exposes **Max Frames** and **Include Audio**, which apply only to video files. A video passed to **Embed Audio** uses 16 sampled frames without its audio track. Use **Prepare Embedding Media** followed by **Embed Content** when you need decoded content for a mixed input or want to reuse it. Audio retains its sample rate, channel layout, and gaps in its timeline. MP4 audio offsets and trims keep the audio aligned with the video. Video carries timestamped image frames and can include its audio track.

The file decoder requires ordered presentation timestamps. H.264 can use a software decoder when the host decoder is unavailable, including buffered frames with verified picture timestamps. Other codecs require one decoded frame per packet. Sources with reordered timestamps, or MP4 edit lists that shift or trim the video track, can supply timestamped frames directly. Keep the original frame timestamps when sampling a clip.

For a hosted model, the media nodes send the original file to the gateway. The gateway samples video frames, so **Max Frames** applies only to local decoding. **Include Audio** supplies the soundtrack separately. To assemble mixed hosted content with **Prepare Embedding Media**, enable **Encoded Source**. You can also pass an `encoded_media` part to **Embed Content**, with a `modality` of `image`, `audio`, or `video` and a `source` containing a public or signed HTTP(S) URL, a base64 data URL, or raw base64. Signed URL query parameters are preserved. One joint item accepts one file per modality; separate items produce separate vectors.

The Internal gateway limits each media file to 100 MiB and a request to 256 MiB of decoded or downloaded media. Audio must be at most 327 seconds. Supply the final download URL: the gateway rejects redirects, private-network addresses, and local file paths. Signed URLs must remain valid through any cold start and the subsequent download. Your hosting platform may impose a smaller HTTP body limit for inline media.

The content nodes return vectors together with their vector-space identity, dimensions, metric, and pipeline fingerprint. Index and query vectors must use a compatible space, dimension count, and metric. Record each encoder's fingerprint to track its weights and preprocessing; paired text and image encoders can have different fingerprints while sharing a vector space. Equal vector lengths alone do not establish model compatibility. New versioned adapters reject inputs that exceed their token context; split long documents before embedding them. Existing Bits retain automatic chunking, and their reported context describes each encoded chunk.

## Decision models

Add a **SystemOne** Bit from the catalog's **Decisions** category to your profile, then connect **Find Decision Model** to the **Model** input of a node under **AI / Decisions**. It selects an available decision model from the active profile at execution time. To use a specific model, connect **Load Bit** instead. The three decision nodes accept text in **State** and a question in **Instructions**:

- **SystemOne Noul** returns a yes/no probability and branches through **True** when it meets the threshold, or **False** otherwise. The threshold defaults to `0.5`.
- **SystemOne Choice** accepts a map of option names to descriptions. It returns the selected name, probabilities, and confidence.
- **SystemOne Score** accepts two to ten level descriptions in order. It returns a weighted level index, probabilities, confidence, and a legend of level descriptions.

The optional **Images** input accepts an array of image objects from image nodes, such as **Read Image**. The node converts them to image data URLs before sending the request.

Use **Invoke SystemOne** to ask several named questions together or supply structured state or criteria. Its request contains `state` and `questions`:

```json
{
  "state": "I was charged twice for my order.",
  "questions": {
    "team": {
      "type": "choice",
      "instructions": "Which team should handle this message?",
      "criteria": {"billing": null, "shipping": null, "technical": null}
    },
    "refund": {
      "type": "noul",
      "instructions": "Does the customer need money returned?"
    }
  }
}
```

`choice` returns the selected option and each option's probability. `score` uses an ordered array of two to ten level descriptions and returns a weighted level index. `noul` returns the probability that a yes/no answer is true. Results arrive together without streaming. Assess these probabilities against examples from your own workflow before choosing an automatic-action threshold.

Local Bits require a native decision-model GGUF supported by the bundled llama.cpp release. Flow-Like starts its server and uses `/v1/systemone`. An ordinary chat GGUF cannot serve this endpoint. Image input requires a supported decision model and its projector. When using **Invoke SystemOne**, supply images as data URLs in the request's `images` array.

Hosted Bits support OpenRouter, Cloudflare Workers AI, TypeSafe, and operator-configured SystemOne services through the Flow-Like API. Hosted calls use the same account authorization, usage tracking, and billing controls as hosted chat. Administrators configure service credentials on the server; Bits identify the provider and model. For an independently managed endpoint, a custom SystemOne Bit uses its own endpoint and credential.

For a device deployment, the SystemOne Bit must be listed in the project's `bits` manifest field so export includes its pinned metadata and local model assets. Adding a model to a profile or selecting it on a node does not update this list. The current project settings have no dependency editor, so deployment requires a prepared project manifest. Hosted Bits also need their metadata packaged and hosted-model access approved for the service.

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
