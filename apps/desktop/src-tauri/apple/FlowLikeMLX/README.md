# FlowLikeMLX

Native MLX LLM/VLM inference for Flow-Like on Apple devices.

The package is pinned to:

- `mlx-swift-lm` revision `10e0cb7442920d3f67a08e067d6670334e9dadef`
  (3.31.4 plus the upstream Gemma 4 VLM shared-KV loader and cooperative
  cancellation fixes)
- `mlx-swift` 0.31.4
- `swift-transformers` 1.3.0
- macOS 14 and iOS 17

## Architecture

Model files remain owned by Flow-Like's Bit download logic. An MLX LLM or VLM
Bit has one dependency per repository asset; Rust verifies those dependencies
and materializes them as one local Hugging Face/MLX model directory. The native
layer receives only that directory and never downloads model weights.

The two Apple hosts use the same `FlowLikeMLX` library:

- iOS links the static library into the app and calls its `@_cdecl` C ABI
  in-process. There is no Swift helper process or Swift network listener on
  iOS. Rust may still expose an in-process loopback compatibility endpoint so
  the existing provider stack can use its normal OpenAI-compatible transport.
- macOS launches the `flow-like-mlx` executable as a private child process and
  exchanges newline-delimited JSON over stdin/stdout. It does not open a TCP
  port. The SwiftPM target/scheme is `FlowLikeMLXServer`; its executable product
  is `flow-like-mlx`.

Both paths return OpenAI-compatible Chat Completions response/chunk payloads to
the Rust compatibility proxy.

## Native runtime behavior

- LLMs load through `LLMModelFactory.shared`; VLMs load through
  `VLMModelFactory.shared`.
- Tokenizers load from the same local Bit directory.
- One generation runs at a time. macOS retains up to two loaded models and iOS
  retains one, using an LRU policy.
- iOS sets the MLX recyclable-buffer cache to 20 MiB, based on the official MLX
  iOS guidance, and the app target requests Apple's increased-memory-limit
  entitlement. On the first memory warning, MLX stops retaining returned
  buffers and releases the model at safe idle; repeated pressure cancels the
  active request. Leaving the foreground closes admission immediately and
  cancels active/queued MLX work until the app becomes active again.
- LLM history uses MLX's raw message representation so `tool_calls` and
  `tool_call_id` survive subsequent tool-loop turns.
- Flow-Like's structured multimodal `Chat.Message` mapping does not yet represent
  tool-call history; VLM requests report that limitation explicitly.
- VLM images accept local files, base64 image data URLs, and HTTP(S). Remote
  responses have finite timeouts, must be successful `image/*` responses, and
  are capped at 20 MiB while streaming.

Only curated models supported by the pinned MLX LLM/VLM registries should be
published as MLX Bits. A directory must include `config.json`, tokenizer files,
and at least one `.safetensors` file; VLMs also require their processor config.

## iOS C ABI contract

Rust imports:

```text
flow_like_mlx_is_available() -> int32
flow_like_mlx_generate(json, callback, context) -> int32
flow_like_mlx_cancel(request_id)
flow_like_mlx_unload(model_directory)
flow_like_mlx_clear_cache()
```

`flow_like_mlx_generate` has an ownership-sensitive contract:

- A nonzero result invokes no callback; Rust still owns `context`.
- A zero result emits serialized events and exactly one terminal `complete` or
  `error`, including cancellation.
- No callback occurs after the terminal callback returns. Rust frees `context`
  during that terminal callback.

The simulator reports MLX unavailable because the iOS simulator lacks the Metal
GPU features required by MLX.

## macOS NDJSON protocol

Each input line is a `FlowLikeMLXCommand`:

```json
{"id":"1","command":"generate","model_directory":"/path/to/model","model_kind":"llm","request":{"messages":[{"role":"user","content":"Hello"}],"stream":true}}
```

Output lines have this envelope:

```json
{"id":"1","event":"chunk","data":{"object":"chat.completion.chunk"}}
{"id":"1","event":"complete","data":{"object":"chat.completion"}}
```

Lifecycle commands `cancel`, `unload`, and `clear_cache` are also accepted.

## Building

MLX includes Metal sources, so build the macOS executable with Xcode rather
than `swift build`:

```sh
cd apps/desktop/src-tauri/apple/FlowLikeMLX
xcodebuild \
  -scheme FlowLikeMLXServer \
  -destination 'platform=macOS,arch=arm64' \
  -configuration Release \
  -derivedDataPath .build/xcode \
  build
```

The resulting product is
`.build/xcode/Build/Products/Release/flow-like-mlx`.
Flow-Like's desktop preparation script copies it to the Tauri sidecar location.

The iOS Xcode project references this package at
`../../apple/FlowLikeMLX` and links the `FlowLikeMLX` library product. Xcode
resolves and compiles MLX, including its Metal resources, as part of the app.

## Tests

The package tests cover request mapping, LLM tool history, VLM limitations,
image reference validation, stop-sequence filtering, generation parameters,
memory-warning/lifecycle admission policy, and terminal callback gating. Run
them from Xcode on an Apple-silicon Mac. Model-backed smoke tests should use a
small curated Bit and a physical iOS device; the unit tests do not download
weights.
