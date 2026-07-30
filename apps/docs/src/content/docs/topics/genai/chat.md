---
title: Chat & Conversations
description: Build multi-turn chat workflows with history, streaming, sessions, and attachments
sidebar:
  order: 3
---

Flow-Like chat boards begin with a **Chat Event** and return responses through chat callback nodes. The event supplies the current conversation and session context; the workflow decides which model, tools, retrieval, and response behavior to apply.

## Chat event contract

[Chat Event](/nodes/events/events-chat/) starts the board and exposes:

| Output | Purpose |
|--------|---------|
| History | Current model-compatible chat history |
| Local Session | State scoped to this chat |
| Global Session | State scoped to the user |
| Tools | Tools requested through the chat surface |
| Actions | User actions from prior interactive content |
| Attachments | Uploaded files or references |
| User | Current user information |

Read only the outputs the workflow needs. Treat user text, attachments, and restored session values as untrusted input.

## Minimal model response

A basic model-backed chat board:

1. receives **History** from Chat Event;
2. applies the system instruction;
3. invokes a configured model;
4. pushes the resulting response to the chat.

| Stage | Node |
|-------|------|
| Set role and rules | [Set System Message](/nodes/ai/generative/history/ai-generative-set-system-prompt-message/) |
| Invoke configured model | [Invoke Model](/nodes/ai/generative/ai-generative-invoke/) |
| Return complete response | [Push Response](/nodes/events/chat/events-chat-push-response/) |

The system message should state the role, source policy, tool policy, refusal behavior, and expected response shape. Do not include credentials or private operational data.

## Work with history

The event's History output already contains the conversation supplied by the chat surface. History nodes let the workflow modify a copy before invocation:

| Need | Node |
|------|------|
| Create an empty history | [Make History](/nodes/ai/generative/history/ai-generative-make-history/) |
| Add a message | [Push Message](/nodes/ai/generative/history/ai-generative-add-history-message/) |
| Build from messages | [From Messages](/nodes/ai/generative/history/ai-generative-from-messages/) |
| Limit retained messages | [Set History N](/nodes/ai/generative/history/ai-generative-set-history-n/) |
| Remove the latest message | [Pop Message from History](/nodes/ai/generative/history/ai-generative-pop-history-message/) |
| Clear history | [Clear History](/nodes/ai/generative/history/ai-generative-clear-history/) |

Keep enough history for the task, but avoid unbounded transcripts. Store durable application facts in typed app or workflow state rather than relying on old conversational text.

## Configure model behavior

History configuration nodes expose settings such as:

- [Set Max Tokens](/nodes/ai/generative/history/ai-generative-set-history-max-tokens/);
- [Set History Temperature](/nodes/ai/generative/history/ai-generative-set-history-temperature/);
- [Set History Top P](/nodes/ai/generative/history/ai-generative-set-history-top-p/);
- [Set Stop Words](/nodes/ai/generative/history/ai-generative-set-history-stop-words/);
- [Set Seed](/nodes/ai/generative/history/ai-generative-set-history-seed/);
- [Set Response Format](/nodes/ai/generative/history/ai-generative-set-history-response-format/);
- [Set Stream](/nodes/ai/generative/history/ai-generative-set-history-stream/).

Provider support varies. A setting present in the workflow does not guarantee that every provider or model implements it identically.

## Stream responses

Enable streaming on the history or invocation path supported by the configured model, then pass each model response chunk to [Push Chunk](/nodes/events/chat/events-chat-push-response-chunk/).

Use [Push Response](/nodes/events/chat/events-chat-push-response/) when:

- the model returns one complete response;
- the workflow must validate or redact the full result before display;
- the response is short enough that progressive delivery adds little value.

Use **Push Chunk** when:

- the model provides response chunks;
- the user benefits from lower perceived latency;
- the workflow can still report a final completion or failure state.

Do not stream raw internal reasoning. Stream user-facing answer content and named progress states.

## Show progress and metadata

The chat callback nodes include:

| Need | Node |
|------|------|
| Add a named progress step | [Push Step](/nodes/events/chat/events-chat-push-step/) |
| Update text for a step | [Push Text to Step](/nodes/events/chat/events-chat-push-text-to-step/) |
| Remove a step | [Remove Step](/nodes/events/chat/events-chat-remove-step/) |
| Push one statistic | [Push Stat](/nodes/events/chat/events-chat-push-stat/) |
| Push several statistics | [Push Stats](/nodes/events/chat/events-chat-push-stats/) |

Progress should describe observable work such as “Searching documents” or “Validating order,” not hidden chain-of-thought.

## Session state

[Push Local Session](/nodes/events/chat/events-chat-push-local-session/) updates state local to the conversation. [Push Global Session](/nodes/events/chat/events-chat-push-global-session/) updates user-level state.

Use local session state for turn-specific workflow context and global session state only for durable user preferences or facts that legitimately cross conversations. Define retention, schema, and deletion behavior for both.

Never trust a restored session field as authorization. Re-check permission at the operation boundary.

## Attachments

[Extract Attachments](/nodes/events/chat/ai-gen-llm-history-extract-attachments/) retrieves attachments from model history. Response attachments can be created:

- [From Path](/nodes/events/chat/attachments/events-chat-attachment-from-path/);
- [From Signed URL](/nodes/events/chat/attachments/events-chat-attachment-from-signed-url/);
- and returned with [Push Attachment](/nodes/events/chat/events-chat-push-attachment/) or [Push Attachments](/nodes/events/chat/events-chat-push-attachments/).

Before processing an upload:

- validate file type and size;
- inspect or scan content according to the app's threat model;
- keep the original source reference;
- send only the necessary content to a model;
- avoid exposing filesystem paths in the response.

## Structured interactions

Use a structured interaction when required input should not be guessed from free text:

| Interaction | Node |
|-------------|------|
| Form | [Chat Form](/nodes/events/chat/interaction/interaction-form/) |
| One option | [Single Choice](/nodes/events/chat/interaction/interaction-single-choice/) |
| Several options | [Multiple Choice](/nodes/events/chat/interaction/interaction-multiple-choice/) |

Structured interactions are useful for configuration, approvals, and collecting missing fields before a tool call.

## Add retrieval or tools

- Use [RAG and knowledge bases](/topics/genai/rag/) when answers must be grounded in a controlled document corpus.
- Use [AI agents](/topics/genai/agents/) when the assistant must choose among bounded tools.
- Keep ordinary deterministic checks in the board even when a model or agent is involved.

## Error handling

Every chat path should finish in one of three visible states:

- complete response;
- request for missing information;
- explicit failure or inability to answer.

If a tool or side effect fails, do not let an earlier streamed sentence imply success. Redact secrets and sensitive payloads from errors, and preserve a safe run identifier for support.

## Production checklist

- [ ] Chat Event outputs are treated as untrusted input
- [ ] System instructions and model configuration are versioned
- [ ] History is bounded
- [ ] Session state has scope and retention rules
- [ ] Streaming has explicit completion and failure
- [ ] Attachments are validated before processing
- [ ] Structured interactions collect required fields
- [ ] Retrieval and tools enforce access outside the prompt
- [ ] Logs and errors redact sensitive data

## Next steps

- [RAG and knowledge bases](/topics/genai/rag/)
- [AI agents](/topics/genai/agents/)
- [Extraction and structured output](/topics/genai/extraction/)
- [Building chatbots](/topics/chatbots/overview/)
