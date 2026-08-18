// AI — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === AI/Agents ===

/**
 * Executes an Agent with history and returns the complete response
 * @param agent — Configured Agent object with tools
 * @param history — Conversation history to provide context
 * @returns response — Final agent response
 * @returns historyOut — Updated conversation history with agent turns
 * @returns stats — Token usage, cost, and model statistics
 * @impure has side effects / drives control flow
 */
declare function agentInvoke({ agent: Struct, history: Struct }): { response: Struct, historyOut: Struct, stats: Struct };

/**
 * Executes an Agent with streaming, emitting chunks in real-time
 * @param agent — Configured Agent object with tools
 * @param history — Conversation history to provide context
 * @returns chunk — Latest streamed chunk from agent response
 * @returns response — Final complete agent response
 * @returns historyOut — Updated conversation history with all agent turns
 * @returns stats — Token usage, cost, and model statistics
 * @impure has side effects / drives control flow
 */
declare function agentStreamInvoke({ agent: Struct, history: Struct }): { chunk: Struct, response: Struct, historyOut: Struct, stats: Struct };

/**
 * LLM-driven control loop that repeatedly calls referenced Flow functions as tools until it decides to stop
 * @param model — Bit describing the LLM that powers the agent
 * @param history — Conversation history shared with the agent (used for reasoning context)
 * @param maxIter (optional) — Maximum number of internal iterations/tool calls before failing
 * @param infiniteContext (optional) — Enable automatic context window management to prevent overflow
 * @param maxContextTokens (optional) — Maximum tokens to retain when truncating (default: 32000)
 * @param contextMode (optional) — How to handle context overflow: 'truncate' (drop old messages) or 'summarize' (LLM compresses history)
 * @returns chunk — Latest streamed agent chunk (final response)
 * @returns response — Final assistant response produced when the agent halts
 * @returns historyOut — Conversation history enriched with all agent/tool turns
 * @returns stats — Token usage, cost, and model statistics
 * @impure has side effects / drives control flow
 */
declare function simpleAgent({ model: Struct, history: Struct, maxIter?: int, infiniteContext?: bool, maxContextTokens?: int, contextMode?: string }): { chunk: Struct, response: Struct, historyOut: Struct, stats: Struct };


// === AI/Agents/Builder ===

/**
 * Add a DataFusion SQL session to an agent for data analysis capabilities
 * @param agent — Agent to add DataFusion context to
 * @param session — DataFusion session from CreateDataFusionSession node
 * @param description — User-friendly description of this data source
 * @param tableDescriptions — Map of table names to descriptions (JSON object)
 * @param exampleQueries — Example SQL queries that work with this data
 * @param discoverSchemas (optional) — Automatically discover table schemas at runtime
 * @returns agentOut — Agent with DataFusion context added
 * @impure has side effects / drives control flow
 */
declare function addDatafusionToAgent({ agent: Struct, session: Struct, description: string, tableDescriptions: Struct, exampleQueries: any, discoverSchemas?: bool }): Struct;

/**
 * Creates an Agent object from a model Bit with configuration
 * @param model — LLM model Bit that will power the agent
 * @param maxIter (optional) — Maximum number of tool call iterations before stopping
 * @param infiniteContext (optional) — Enable automatic context window management to prevent overflow
 * @param contextMode (optional) — Strategy: 'truncate' (fast, drops old messages) or 'summarize' (LLM compresses history, slower but preserves info)
 * @param maxContextTokens (optional) — Maximum tokens to retain in context window (default: 32000)
 * @returns agentOut — Configured Agent object ready for tool registration and execution
 */
declare function agentFromModel({ model: Struct, maxIter?: int, infiniteContext?: bool, contextMode?: string, maxContextTokens?: int }): Struct;

/**
 * Indexes referenced Flow-Like functions into a vector DB so agents can discover tools via semantic search at runtime, keeping the context window lean.
 * @param agentIn — Agent object to register lazy function tools on
 * @param model — Embedding model used to index functions for semantic search
 * @returns agentOut — Agent with lazy function tool references attached
 * @impure has side effects / drives control flow
 */
declare function agentLazyRegisterFunctionTools({ agentIn: Struct, model: Struct }): Struct;

/**
 * Adds referenced Flow-Like functions as callable tool references to an Agent
 * @param agentIn — Agent object to add function references to
 * @returns agentOut — Agent object with registered function tool references
 */
declare function agentRegisterFunctionTools({ agentIn: Struct }): Struct;

/**
 * Adds Model Context Protocol (MCP) server tools to an Agent
 * @param agentIn — Agent object to add MCP tools to
 * @param uri — URI of the MCP server to connect to
 * @param mode (optional) — How to select MCP tools (Automatic = all tools, Manual = pick specific tools)
 * @returns agentOut — Agent object with registered MCP tools
 */
declare function agentRegisterMcpTools({ agentIn: Struct, uri: string, mode?: string }): Struct;

/**
 * Gives the agent autonomous access to persistent memory tools (_memory_search, _memory_store, _memory_compress)
 * @param agentIn — Agent object to register memory on
 * @param memoryConfig — MemoryConfig from Create Memory Config node (bundles database + embedding model + tuning parameters)
 * @returns agentOut — Agent with memory tools registered
 */
declare function agentRegisterMemory({ agentIn: Struct, memoryConfig: Struct }): Struct;

/**
 * Adds a connected app's MCP event as agent tools. Uses a short-lived app-to-app token (valid ~15 minutes) that is refreshed on every run.
 * @param agentIn — Agent object to add the remote MCP tools to
 * @param flowRemoteAppId (optional) — Connected project that hosts the MCP event
 * @param flowRemoteEvent (optional) — MCP event of the selected project
 * @param flowRemoteEventMeta (optional) — Auto-filled by the editor when an event is selected
 * @param toolFilter — Optional list of tool names to include. Empty = all tools.
 * @param headers — Static registration authentication headers (for example Authorization or x-api-key). HMAC auth is not supported because each MCP request requires a fresh signature.
 * @returns agentOut — Agent object with the remote MCP tools registered
 */
declare function agentRegisterRemoteMcpTools({ agentIn: Struct, flowRemoteAppId?: string, flowRemoteEvent?: string, flowRemoteEventMeta?: string, toolFilter: string[], headers: Struct }): Struct;

/**
 * Enables Rig's built-in Thinking tool for reasoning capabilities
 * @param agentIn — Agent object to enable thinking on
 * @returns agentOut — Agent object with thinking tool enabled
 */
declare function agentRegisterThinking({ agentIn: Struct }): Struct;

/**
 * Sets the system prompt for an Agent to guide its behavior
 * @param agentIn — Agent object to enable thinking on
 * @param systemPrompt (optional) — System prompt string to set for the agent
 * @returns agentOut — Agent object with thinking tool enabled
 */
declare function agentSetSystemPrompt({ agentIn: Struct, systemPrompt?: string }): Struct;

/**
 * Registers a knowledge graph traversal tool on the agent so it can query the graph mid-conversation
 * @param agentIn — Agent to register the KG tool on
 * @param graph — Graph connection from Open Graph Overlay node
 * @param toolName (optional) — Name for the registered tool (shown to the LLM)
 * @param toolDescription (optional) — Description of the tool for the LLM
 * @returns agentOut — Agent with the KG traverse tool registered
 */
declare function kgTraverseTool({ agentIn: Struct, graph: Struct, toolName?: string, toolDescription?: string }): Struct;


// === AI/Embedding ===

/**
 * Creates an embedding vector for a document string using a cached embedding model
 * @param queryString — Document text that should be embedded
 * @param model — Cached embedding Bit containing the provider
 * @returns vector — Embedding vector returned by the model
 * @impure has side effects / drives control flow
 */
declare function embedDocument({ queryString: string, model: Struct }): float[];

/**
 * Embeds an image using a loaded model
 * @param image — The image to embed
 * @param model — The embedding model
 * @returns vector — The embedding vector
 * @impure has side effects / drives control flow
 */
declare function embedImage({ image: Struct, model: Struct }): float[];

/**
 * Embeds a query string using a loaded model
 * @param queryString — The string to embed
 * @param model — The embedding model
 * @returns vector — The embedding vector
 * @impure has side effects / drives control flow
 */
declare function embedQuery({ queryString: string, model: Struct }): float[];

/**
 * Loads a model from a Bit
 * @param bit — The Bit that contains the Model
 * @returns model — Model Out
 * @impure has side effects / drives control flow
 */
declare function loadModel({ bit: Struct }): Struct;


// === AI/Generative ===

/**
 * Adds custom HTTP headers to a model for use with custom API endpoints
 * @param model — Model to add headers to
 * @param header (optional) — HTTP header to add (name-value pair)
 * @returns modelOut — Model with custom headers applied
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeAddHeaders({ model: Struct, header?: Struct }): Struct;

/**
 * Finds the best model based on certain selection criteria
 * @param preferences (optional) — Weights and requirements that guide model selection
 * @returns model — Bit describing the best-match model
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeFindModel({ preferences?: Struct }): Struct;

/**
 * Invokes the configured model with the provided chat history. Set history streaming off to preserve and replay structured media responses.
 * @param model — Model
 * @param history — Chat History
 * @returns chunk
 * @returns result — Resulting Model Output
 * @returns stats — Token usage, cost, and model statistics
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeInvoke({ model: Struct, history: Struct }): { chunk: Struct, result: Struct, stats: Struct };

/**
 * Invokes an LLM with a system prompt and user prompt, returning text and the full structured response.
 * @param model — Bit describing the provider/model to execute
 * @param systemPrompt (optional) — Optional system instructions to prime the assistant
 * @param prompt (optional) — User message that will be sent to the model
 * @param stream (optional) — Stream text tokens when possible. Disable to preserve structured media responses and replay them as rich chunks.
 * @returns token — Most recently streamed token or chunk
 * @returns chunk — Most recent structured stream or replay chunk, including media content parts
 * @returns result — Final assistant message extracted from the response
 * @returns response — Full structured model response, including media content parts and reasoning
 * @returns stats — Token usage, cost, and model statistics
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeInvokeSimple({ model: Struct, systemPrompt?: string, prompt?: string, stream?: bool }): { token: string, chunk: Struct, result: string, response: Struct, stats: Struct };

/**
 * Summarizes long text using an LLM with configurable strategies. Supports Map-Reduce (parallel, fast), Refine (sequential, coherent), Hierarchical (structure-aware), Hybrid (parallel + coherent), and Sliding Window (memory-efficient). Optional Chain of Density post-processing for optimal information density.
 * @param model — Bit describing the provider/model to use for summarization
 * @param text (optional) — The long text to summarize (markdown supported)
 * @param strategy (optional) — Summarization strategy: • Refine — sequential, best coherence, no parallelism • MapReduce — parallel chunking, fast, may lose cross-chunk context • Hierarchical — structure-aware tree, best for headed documents • Hybrid — MapReduce speed + Refine coherence polish • SlidingWindow — fixed memory buffer, best for very long documents
 * @param densification (optional) — Post-processing to increase information density: • None — use the strategy output as-is • ChainOfDensity — iteratively compress to optimal density (~0.15 entities/token)
 * @param instructions (optional) — Optional focus instructions (e.g. 'focus on action items', 'use bullet points')
 * @param priorSummary (optional) — Optional existing summary to build upon (used as initial context for Refine/Hybrid/SlidingWindow strategies)
 * @param chunkSize (optional) — Maximum characters per chunk. Reduce for models with smaller context windows (default: 8000)
 * @param chunkOverlap (optional) — Overlap between adjacent chunks as percentage (0-50). Prevents information loss at boundaries (default: 10)
 * @param trackEntities (optional) — Extract and track named entities across chunks to prevent information loss. Adds 2-3 extra LLM calls but improves factual preservation.
 * @param concurrency (optional) — Parallel requests for MapReduce/Hybrid strategies. 0 = unlimited, 1 = sequential (default: 4)
 * @param maxIterations (optional) — Safety limit on summarization passes. Each pass reduces total length (default: 5)
 * @param densitySteps (optional) — Number of Chain of Density refinement steps when densification is enabled (1-5, default: 3). Research shows step 3 is the human-preferred sweet spot.
 * @returns summary — The final summarized text
 * @returns entities — Tracked entities found in the document (only populated when Track Entities is enabled)
 * @returns llmCalls — Total number of LLM invocations used during summarization
 * @impure has side effects / drives control flow
 */
declare function aiLlmSummarize({ model: Struct, text?: string, strategy?: string, densification?: string, instructions?: string, priorSummary?: string, chunkSize?: int, chunkOverlap?: int, trackEntities?: bool, concurrency?: int, maxIterations?: int, densitySteps?: int }): { summary: string, entities: string[], llmCalls: int };

/**
 * Invokes an LLM that can call Flow tools/functions and routes each call to execution pins.
 * @param model — Bit describing the provider/model to execute
 * @param history — Conversation history the model should continue from
 * @param tools (optional) — JSON array of tool/function definitions (OpenAI format)
 * @param toolChoice (optional) — Controls whether the model must, may, or must not call tools
 * @returns response — LLM response if the model answered directly without tool calls
 * @returns toolCallArgs — Parsed JSON arguments for the latest tool call
 * @returns stats — Token usage, cost, and model statistics
 * @impure has side effects / drives control flow
 */
declare function invokeLlmWithTools({ model: Struct, history: Struct, tools?: string, toolChoice?: string }): { response: Struct, toolCallArgs: Struct, stats: Struct };

/**
 * Routes execution based on an LLM-evaluated yes/no decision
 * @param model — Bit representing the LLM to query
 * @param prompt — Statement/question that should result in a yes/no decision
 * @impure has side effects / drives control flow
 */
declare function llmBranch({ model: Struct, prompt: string }): void;

/**
 * Uses an LLM plus a JSON schema to extract structured data from free-form text
 * @param model — Bit pointing to the LLM that will perform the extraction
 * @param schema — JSON Schema (or example JSON) describing the structure to extract
 * @param text — Raw text that should be structured via the schema
 * @param hint (optional) — Optional hint to guide the extraction (e.g. 'only extract individual line items, not totals')
 * @returns response — Structured JSON value that matches the schema
 * @returns stats — Token usage, cost, and model statistics
 * @impure has side effects / drives control flow
 */
declare function llmExtractor({ model: Struct, schema: string, text: string, hint?: string }): { response: any, stats: Struct };

/**
 * Extracts structured data by replaying an entire chat history through an LLM
 * @param model — Bit pointing to the LLM that will perform the extraction
 * @param schema — JSON Schema (or example JSON) describing the structure to extract
 * @param history — Chat history to replay when extracting data
 * @param hint (optional) — Optional hint to guide the extraction (e.g. 'only extract individual line items, not totals')
 * @returns response — Structured JSON value that matches the schema
 * @returns stats — Token usage, cost, and model statistics
 * @impure has side effects / drives control flow
 */
declare function llmExtractorHistory({ model: Struct, schema: string, history: Struct, hint?: string }): { response: any, stats: Struct };


// === AI/Generative/Audio ===

/**
 * Transcribes audio locally with an installed any-speech-to-text model bit. Decodes WAV, MP3, FLAC, OGG (Vorbis/Opus), WebM/Opus, M4A/MP4 (AAC) and PCM, including browser MediaRecorder output (Chrome WebM/Opus, Safari MP4/AAC).
 * @param bit — Installed local STT model Bit
 * @param audio — Audio file path. WAV, MP3, FLAC, OGG (Vorbis/Opus), WebM/Opus, M4A/MP4 (AAC) and PCM are decoded automatically, including browser MediaRecorder recordings.
 * @param language (optional) — Optional source language code. Use auto to detect.
 * @param task (optional) — Transcribe in the source language or translate to English
 * @param timestamps (optional) — Emit per-segment timestamps in the metadata
 * @returns text — Transcript text
 * @returns message — Transcript as a user HistoryMessage
 * @returns history — Transcript wrapped in History
 * @returns metadata — Local transcription metadata (model, runtime, detected language, duration, and segments)
 * @impure has side effects / drives control flow
 */
declare function aiAudioLocalSpeechToText({ bit: Struct, audio: Struct, language?: string, task?: string, timestamps?: bool }): { text: string, message: Struct, history: Struct, metadata: Struct };

/**
 * Generates WAV speech locally with an installed any-tts model bit.
 * @param bit — Installed TTS model Bit
 * @param text (optional) — Text to synthesize
 * @param outputPath — Destination FlowPath for generated WAV audio
 * @param language (optional) — Optional language code or name. Use auto for model default.
 * @param voice (optional) — Optional voice or speaker name. Use auto for model default.
 * @param instruct (optional) — Optional style instruction for models that support it
 * @param maxTokens (optional) — Optional generation token limit. Use 0 for model default.
 * @param temperature (optional) — Optional sampling temperature. Use 0 for model default.
 * @param cfgScale (optional) — Optional guidance scale. Use 0 for model default.
 * @param referenceAudio (optional) — Optional FlowPath to WAV or MP3 reference audio for voice cloning
 * @returns path — Generated WAV path
 * @returns metadata — Local synthesis metadata
 * @impure has side effects / drives control flow
 */
declare function aiAudioLocalTextToSpeech({ bit: Struct, text?: string, outputPath: Struct, language?: string, voice?: string, instruct?: string, maxTokens?: int, temperature?: float, cfgScale?: float, referenceAudio?: Struct }): { path: Struct, metadata: Struct };

/**
 * Transcribes or translates audio with an existing provider Bit.
 * @param provider — Existing provider Bit
 * @param audio — Audio FlowPath
 * @param providerOptions (optional) — Typed provider-specific speech-to-text options
 * @returns text — Transcript text
 * @returns message — Transcript as a user HistoryMessage
 * @returns history — Transcript wrapped in History
 * @returns metadata — Transcription metadata
 * @impure has side effects / drives control flow
 */
declare function aiAudioSpeechToText({ provider: Struct, audio: Struct, providerOptions?: Struct }): { text: string, message: Struct, history: Struct, metadata: Struct };

/**
 * Generates speech audio with an existing provider Bit and writes it to FlowPath.
 * @param provider — Existing provider Bit
 * @param text (optional) — Text to synthesize
 * @param outputPath — Destination FlowPath for generated audio
 * @param providerOptions (optional) — Typed provider-specific text-to-speech options
 * @returns path — Generated audio path
 * @returns metadata — Generation metadata
 * @impure has side effects / drives control flow
 */
declare function aiAudioTextToSpeech({ provider: Struct, text?: string, outputPath: Struct, providerOptions?: Struct }): { path: Struct, metadata: Struct };


// === AI/Generative/Audio/Options ===

/**
 * Creates typed speech-to-text options for Gemini and Vertex audio transcription.
 * @param prompt (optional) — Transcription instruction prompt
 * @returns options — Typed speech-to-text provider options
 */
declare function aiAudioSttOptionsGoogle({ prompt?: string }): Struct;

/**
 * Creates typed speech-to-text options for OpenAI-compatible providers.
 * @param prompt (optional) — Optional transcription prompt or context
 * @param language (optional) — Optional source language code
 * @param responseFormat (optional) — Provider response format
 * @param translate (optional) — Translate audio to English when the provider supports it
 * @returns options — Typed speech-to-text provider options
 */
declare function aiAudioSttOptionsOpenaiCompatible({ prompt?: string, language?: string, responseFormat?: string, translate?: bool }): Struct;

/**
 * Creates typed speech-to-text options for xAI transcription models.
 * @param prompt (optional) — Optional transcription prompt or context
 * @param language (optional) — Optional source language code
 * @returns options — Typed speech-to-text provider options
 */
declare function aiAudioSttOptionsXai({ prompt?: string, language?: string }): Struct;

/**
 * Creates typed text-to-speech options for Gemini and Vertex speech models.
 * @param voice (optional) — Google prebuilt voice name
 * @param instructions (optional) — Optional style or delivery instructions
 * @param language (optional) — Optional BCP-47 language code
 * @param outputFormat (optional) — Requested output audio format
 * @returns options — Typed text-to-speech provider options
 */
declare function aiAudioTtsOptionsGoogle({ voice?: string, instructions?: string, language?: string, outputFormat?: string }): Struct;

/**
 * Creates typed text-to-speech options for Hugging Face speech models.
 * @param voice (optional) — Optional voice parameter
 * @param outputFormat (optional) — Requested output audio format
 * @param speed (optional) — Playback speed multiplier. Use 0 for provider default.
 * @returns options — Typed text-to-speech provider options
 */
declare function aiAudioTtsOptionsHuggingface({ voice?: string, outputFormat?: string, speed?: float }): Struct;

/**
 * Creates typed text-to-speech options for Mistral speech models.
 * @param voice (optional) — Mistral voice identifier
 * @param outputFormat (optional) — Requested output audio format
 * @returns options — Typed text-to-speech provider options
 */
declare function aiAudioTtsOptionsMistral({ voice?: string, outputFormat?: string }): Struct;

/**
 * Creates typed text-to-speech options for OpenAI-compatible providers.
 * @param voice (optional) — Provider voice identifier
 * @param instructions (optional) — Optional style or delivery instructions
 * @param outputFormat (optional) — Requested output audio format
 * @param speed (optional) — Playback speed multiplier. Use 0 for provider default.
 * @returns options — Typed text-to-speech provider options
 */
declare function aiAudioTtsOptionsOpenaiCompatible({ voice?: string, instructions?: string, outputFormat?: string, speed?: float }): Struct;

/**
 * Creates typed text-to-speech options for xAI speech models.
 * @param voice (optional) — xAI voice identifier
 * @param language (optional) — Optional language code
 * @param outputFormat (optional) — Requested output audio codec
 * @param sampleRate (optional) — Optional output sample rate. Use 0 for provider default.
 * @param bitRate (optional) — Optional MP3 bit rate. Use 0 for provider default.
 * @returns options — Typed text-to-speech provider options
 */
declare function aiAudioTtsOptionsXai({ voice?: string, language?: string, outputFormat?: string, sampleRate?: int, bitRate?: int }): Struct;


// === AI/Generative/History ===

/**
 * Appends a chat message to the end of a history
 * @param history — Chat history to append to
 * @param message — Message that should be appended
 * @returns historyOut — History including the new message
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeAddHistoryMessage({ history: Struct, message: Struct }): Struct;

/**
 * Clears all messages from a ChatHistory
 * @param history — ChatHistory
 * @returns historyOut — Cleared ChatHistory
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeClearHistory({ history: Struct }): Struct;

/**
 * Creates a Chat History from Messages
 * @param modelName (optional) — Model Name
 * @param messages — Chat Messages
 * @returns history — ChatHistory
 */
declare function aiGenerativeFromMessages({ modelName?: string, messages: Struct[] }): Struct;

/**
 * Extracts the first system-level message from a chat history for downstream use
 * @param history — Chat history that contains the system prompt
 * @returns systemPrompt — Extracted system-level message
 * @returns success — True when a system message was located
 */
declare function aiGenerativeGetSystemPrompt({ history: Struct }): { systemPrompt: Struct, success: bool };

/**
 * Creates a ChatHistory Struct from String (as User Message)
 * @param modelName (optional) — Model Name
 * @param message — User Message String
 * @returns history — ChatHistory
 */
declare function aiGenerativeHistoryFromString({ modelName?: string, message: string }): Struct;

/**
 * Creates a ChatHistory struct
 * @param modelName (optional) — Model Name
 * @returns history — ChatHistory
 */
declare function aiGenerativeMakeHistory({ modelName?: string }): Struct;

/**
 * Removes and returns the last message in a chat history
 * @param history — History to remove the message from
 * @returns historyOut — History after removing the message
 * @returns message — Removed message
 * @impure has side effects / drives control flow
 */
declare function aiGenerativePopHistoryMessage({ history: Struct }): { historyOut: Struct, message: Struct };

/**
 * Stores the frequency penalty parameter used by LLM sampling
 * @param history — Existing chat history to update
 * @param frequencyPenalty — Penalty applied when token frequency increases
 * @returns historyOut — History updated with frequency penalty
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistoryFrequencyPenalty({ history: Struct, frequencyPenalty: float }): Struct;

/**
 * Stores the maximum completion tokens allowed for future calls
 * @param history — Existing chat history to update
 * @param maxTokens — Maximum number of completion tokens
 * @returns historyOut — History updated with the max tokens limit
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistoryMaxTokens({ history: Struct, maxTokens: int }): Struct;

/**
 * Stores how many completions to request in downstream LLM calls
 * @param history — Existing chat history to update
 * @param n — Number of completions (u32)
 * @returns historyOut — History including the completion count
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistoryN({ history: Struct, n: int }): Struct;

/**
 * Stores the presence penalty parameter used for discouraging repetition
 * @param history — Existing chat history to update
 * @param presencePenalty — Penalty applied when a token already appeared
 * @returns historyOut — History updated with the presence penalty
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistoryPresencePenalty({ history: Struct, presencePenalty: float }): Struct;

/**
 * Configures the structured response format expected from later LLM calls
 * @param history — Existing chat history to update
 * @param responseFormat — JSON schema or `string` that shapes responses
 * @returns historyOut — History updated with the response format
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistoryResponseFormat({ history: Struct, responseFormat: any }): Struct;

/**
 * Stores an optional randomness seed alongside the chat history
 * @param history — Existing chat history to update
 * @param seed — Deterministic seed value (u32)
 * @returns historyOut — History including the new seed
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistorySeed({ history: Struct, seed: int }): Struct;

/**
 * Stores one or more stop sequences to truncate future completions
 * @param history — Existing chat history to update
 * @param stopWords — Strings that should stop generation
 * @returns historyOut — History updated with stop sequences
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistoryStopWords({ history: Struct, stopWords: string[] }): Struct;

/**
 * Stores whether downstream LLM invocations should stream tokens
 * @param history — Existing chat history to update
 * @param stream (optional) — Whether streaming tokens should be requested
 * @returns historyOut — History updated with the stream setting
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistoryStream({ history: Struct, stream?: bool }): Struct;

/**
 * Stores the sampling temperature used for later LLM invocations
 * @param history — Existing chat history to update
 * @param temperature — Sampling temperature (0-2)
 * @returns historyOut — History including the temperature setting
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistoryTemperature({ history: Struct, temperature: float }): Struct;

/**
 * Stores the thinking level that downstream model invocations should use
 * @param history — Existing chat history to update
 * @param thinking (optional) — Reasoning effort for downstream models: off, low, mid, or high
 * @returns historyOut — History updated with the thinking mode
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistoryThinking({ history: Struct, thinking?: string }): Struct;

/**
 * Stores the nucleus sampling (top-p) parameter alongside the chat history
 * @param history — Existing chat history to update
 * @param topP — Nucleus sampling probability mass (0-1)
 * @returns historyOut — History including the top-p value
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistoryTopP({ history: Struct, topP: float }): Struct;

/**
 * Updates the user identifier stored alongside the chat history
 * @param history — Existing chat history to update
 * @param user — User identifier or label to attach
 * @returns historyOut — History reflecting the new user metadata
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetHistoryUser({ history: Struct, user: string }): Struct;

/**
 * Creates or replaces the system prompt within a chat history before invoking an LLM
 * @param history — Existing chat history to modify
 * @param message (optional) — System-level prompt text
 * @returns historyOut — History including the new system prompt
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetSystemPromptMessage({ history: Struct, message?: string }): Struct;


// === AI/Generative/History/Message ===

/**
 * Creates a chat message with text, image, audio, video, or document content and optional tool metadata
 * @param role (optional) — Author role
 * @param type (optional) — Message content type
 * @param text (optional) — Text content
 * @param image (optional) — Image URL, data URI, file_id reference, or bare base64 payload
 * @param audio (optional) — Audio URL, data URI, file_id reference, or bare base64 payload
 * @param video (optional) — Video URL, data URI, file_id reference, or bare base64 payload
 * @param document (optional) — Document URL, data URI, file_id reference, or bare base64 payload
 * @param detail (optional) — Image resolution detail level
 * @param mime (optional) — Auto infers MIME from a URL or data URI; select a type for bare base64
 * @param toolCallId (optional) — Tool Call Identifier
 * @returns message — Newly constructed chat message
 */
declare function aiGenerativeMakeHistoryMessage({ role?: string, type?: string, text?: string, image?: string, audio?: string, video?: string, document?: string, detail?: string, mime?: string, toolCallId?: string }): Struct;

/**
 * Extracts text content from a chat message, flattening multi-part payloads
 * @param message — Message whose text content will be extracted
 * @returns content — Concatenated text content
 * @returns parts — Ordered text and media content parts
 * @returns images — Image URLs or data URIs
 * @returns audio — Audio URLs or data URIs
 * @returns videos — Video URLs or data URIs
 * @returns documents — Document URLs or data URIs
 */
declare function aiGenerativeMessageExtractContent({ message: Struct }): { content: string, parts: Struct[], images: string[], audio: string[], videos: string[], documents: string[] };

/**
 * Appends text, image, audio, video, or document parts onto a chat message
 * @param message — Message to extend
 * @param type (optional) — Content type
 * @param text (optional) — Text content
 * @param image (optional) — Image URL, data URI, file_id reference, or bare base64 payload
 * @param audio (optional) — Audio URL, data URI, file_id reference, or bare base64 payload
 * @param video (optional) — Video URL, data URI, file_id reference, or bare base64 payload
 * @param document (optional) — Document URL, data URI, file_id reference, or bare base64 payload
 * @param detail (optional) — Image resolution detail level
 * @param mime (optional) — Auto infers MIME from a URL or data URI; select a type for bare base64
 * @returns messageOut — Updated message with additional content
 * @impure has side effects / drives control flow
 */
declare function aiGenerativePushContent({ message: Struct, type?: string, text?: string, image?: string, audio?: string, video?: string, document?: string, detail?: string, mime?: string }): Struct;


// === AI/Generative/Image ===

/**
 * Generates one image with an existing provider Bit and writes it to FlowPath.
 * @param provider — Existing provider Bit
 * @param history — Conversation history. The final user message is used as the image prompt.
 * @param outputPath — Destination FlowPath for generated image output
 * @param providerOptions (optional) — Typed provider-specific image options
 * @returns path — First generated image path
 * @returns paths — All generated image paths
 * @returns metadata — Generation metadata
 * @impure has side effects / drives control flow
 */
declare function aiImageGenerate({ provider: Struct, history: Struct, outputPath: Struct, providerOptions?: Struct }): { path: Struct, paths: Struct[], metadata: Struct };


// === AI/Generative/Image/Options ===

/**
 * Creates typed image options for AWS Bedrock image models.
 * @param aspectRatio (optional) — Bedrock image aspect ratio. Ignored when Size is set.
 * @param size (optional) — Bedrock output size
 * @param quality (optional) — Bedrock image quality
 * @param negativePrompt (optional) — Text describing what to avoid
 * @param seed (optional) — Optional seed. Use 0 for provider default.
 * @param outputFormat (optional) — Requested output image format
 * @returns options — Typed image generation provider options
 */
declare function aiImageOptionsAwsBedrock({ aspectRatio?: string, size?: string, quality?: string, negativePrompt?: string, seed?: int, outputFormat?: string }): Struct;

/**
 * Creates typed image options for Google AI Studio and Vertex Imagen models.
 * @param aspectRatio (optional) — Imagen aspect ratio
 * @param negativePrompt (optional) — Text describing what to avoid
 * @param seed (optional) — Optional seed. Use 0 for provider default.
 * @param outputFormat (optional) — Requested output image format
 * @returns options — Typed image generation provider options
 */
declare function aiImageOptionsGoogleImagen({ aspectRatio?: string, negativePrompt?: string, seed?: int, outputFormat?: string }): Struct;

/**
 * Creates typed image options for Hugging Face text-to-image models.
 * @param size (optional) — Hugging Face output size
 * @param negativePrompt (optional) — Text describing what to avoid
 * @param seed (optional) — Optional seed. Use 0 for provider default.
 * @param outputFormat (optional) — Requested output image format
 * @returns options — Typed image generation provider options
 */
declare function aiImageOptionsHuggingface({ size?: string, negativePrompt?: string, seed?: int, outputFormat?: string }): Struct;

/**
 * Creates typed image options for OpenAI and Azure OpenAI image generation.
 * @param size (optional) — OpenAI image size
 * @param quality (optional) — OpenAI image quality
 * @param background (optional) — OpenAI background behavior
 * @param outputFormat (optional) — Requested output image format
 * @returns options — Typed image generation provider options
 */
declare function aiImageOptionsOpenai({ size?: string, quality?: string, background?: string, outputFormat?: string }): Struct;

/**
 * Creates typed image options for OpenRouter image-output models.
 * @param aspectRatio (optional) — OpenRouter image aspect ratio
 * @param size (optional) — OpenRouter image size
 * @returns options — Typed image generation provider options
 */
declare function aiImageOptionsOpenrouter({ aspectRatio?: string, size?: string }): Struct;

/**
 * Creates typed image options for Together text-to-image models.
 * @param aspectRatio (optional) — Together aspect ratio. Ignored when Size is set.
 * @param size (optional) — Together output size
 * @param negativePrompt (optional) — Text describing what to avoid
 * @param seed (optional) — Optional seed. Use 0 for provider default.
 * @param outputFormat (optional) — Requested output image format
 * @returns options — Typed image generation provider options
 */
declare function aiImageOptionsTogether({ aspectRatio?: string, size?: string, negativePrompt?: string, seed?: int, outputFormat?: string }): Struct;

/**
 * Creates typed image options for xAI image generation.
 * @param aspectRatio (optional) — xAI image aspect ratio
 * @returns options — Typed image generation provider options
 */
declare function aiImageOptionsXai({ aspectRatio?: string }): Struct;


// === AI/Generative/Preferences ===

/**
 * Creates a BitModelPreference struct used to guide model selection
 * @param multimodal (optional) — True if the target model must handle images
 * @returns preferences — Constructed BitModelPreference struct
 */
declare function aiGenerativeMakePreferences({ multimodal?: bool }): Struct;

/**
 * Adds a soft preference hint for downstream model selection
 * @param preferencesIn — Current model preference state
 * @param modelHint — Friendly hint describing the desired model family
 * @returns preferencesOut — Preferences with the new hint
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetModelHint({ preferencesIn: Struct, modelHint: string }): Struct;

/**
 * Adjusts the relative weight for a specific capability preference
 * @param preferencesIn — Current preference struct
 * @param preferencesKey (optional) — Which capability weight to change
 * @param weight — Weight to set
 * @returns preferencesOut — Preferences carrying the new weight
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeSetPreferenceWeight({ preferencesIn: Struct, preferencesKey?: string, weight: float }): Struct;


// === AI/Generative/Provider ===

/**
 * Prepares a Bit for Anthropic's Claude API using the provided credentials
 * @param endpoint (optional) — Anthropic API endpoint
 * @param apiKey (optional) — Anthropic API key
 * @param modelId (optional) — Claude model identifier
 * @returns model — Bit containing the provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildAnthropic({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Builds a model served by Atlas Cloud, a full-modal AI inference platform exposing a single OpenAI-compatible API (DeepSeek, Qwen, GLM, Kimi, MiniMax and more)
 * @param endpoint (optional) — Atlas Cloud OpenAI-compatible base URL (override only for a proxy)
 * @param apiKey (optional) — Atlas Cloud API key used for authentication
 * @param modelId (optional) — Atlas Cloud model identifier to request (e.g., deepseek-ai/deepseek-v4-pro)
 * @returns model — Structured Bit describing the Atlas Cloud provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildAtlascloud({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Prepares a Bit for AWS Bedrock model endpoints
 * @param region (optional) — AWS Bedrock runtime region
 * @param endpoint (optional) — Optional Bedrock Runtime endpoint override. Leave empty to derive from region.
 * @param apiKey (optional) — Credential used for Bedrock runtime requests
 * @param modelId (optional) — AWS Bedrock model identifier
 * @returns model — Structured Bit describing the AWS Bedrock provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildBedrock({ region?: string, endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Prepares a Bit for Cohere's API using the supplied credentials
 * @param endpoint (optional) — Cohere API endpoint (override for private deployments)
 * @param apiKey (optional) — Cohere API key
 * @param modelId (optional) — Cohere model identifier
 * @returns model — Bit containing the provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildCohere({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Prepares a Bit for Deepseek's API using the provided credentials
 * @param endpoint (optional) — Deepseek API endpoint
 * @param apiKey (optional) — Deepseek API key
 * @param modelId (optional) — Deepseek model identifier
 * @returns model — Bit containing the provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildDeepseek({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Prepares a Bit for Galadriel's verified endpoint using the provided credentials
 * @param endpoint (optional) — Galadriel API endpoint
 * @param apiKey (optional) — Galadriel API key
 * @param modelId (optional) — Galadriel model identifier
 * @returns model — Bit containing the provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildGaladriel({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Prepares a Bit for Google Gemini endpoints using the provided credentials
 * @param endpoint (optional) — Gemini REST endpoint
 * @param apiKey (optional) — Gemini API key
 * @param modelId (optional) — Gemini model identifier
 * @returns model — Bit containing the provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildGemini({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Prepares a Bit for Groq's API using the supplied endpoint and key
 * @param endpoint (optional) — Groq-compatible API endpoint
 * @param apiKey (optional) — Groq API key
 * @param modelId (optional) — Groq-served model identifier
 * @returns model — Bit containing the provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildGroq({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Builds the Huggingface model based on certain selection criteria
 * @param endpoint (optional) — Router or custom inference endpoint to use for requests
 * @param apiKey (optional) — Token used for authenticating against the Hugging Face endpoint
 * @param modelId (optional) — Repository/model identifier to load (e.g. meta-llama/Meta-Llama-3-8B-Instruct)
 * @returns model — Structured Bit describing the Hugging Face provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildHuggingface({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Builds the Hyperbolic model based on certain selection criteria
 * @param endpoint (optional) — Public API endpoint or custom proxy to reach Hyperbolic
 * @param apiKey (optional) — Token used for authenticating against Hyperbolic
 * @param modelId (optional) — Repository slug or model identifier to load
 * @returns model — Structured Bit describing the Hyperbolic provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildHyperbolic({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Connects to a locally running LM Studio server via its OpenAI-compatible API
 * @param endpoint (optional) — LM Studio server URL (default: http://localhost:1234)
 * @param modelId (optional) — Model identifier as shown in LM Studio (e.g. lmstudio-community/gemma-3-12b)
 * @returns model — Structured Bit describing the LM Studio provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildLmstudio({ endpoint?: string, modelId?: string }): Struct;

/**
 * Prepares a Bit for the MiniMax API using the provided credentials
 * @param region (optional) — MiniMax API region used when no custom endpoint is provided
 * @param endpoint (optional) — Optional MiniMax API base URL override for a proxy
 * @param apiKey (optional) — MiniMax API key used for authentication
 * @param modelId (optional) — MiniMax model identifier to request
 * @returns model — Bit containing the provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildMinimax({ region?: string, endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Builds the Mira model based on certain selection criteria
 * @param endpoint (optional) — Public Mira API endpoint or private gateway override
 * @param apiKey (optional) — Token used for authenticating against Mira
 * @param modelId (optional) — Model identifier or preset slug to deploy
 * @returns model — Structured Bit describing the Mira provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildMira({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Builds the Mistral model based on certain selection criteria
 * @param endpoint (optional) — Public Mistral endpoint or private deployment URL
 * @param apiKey (optional) — Token used for authenticating against Mistral
 * @param modelId (optional) — Model identifier or preset slug to load
 * @returns model — Structured Bit describing the Mistral provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildMistral({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Builds the Moonshot AI model based on certain selection criteria
 * @param endpoint (optional) — Public Moonshot endpoint or custom proxy URL
 * @param apiKey (optional) — Token used for authenticating against Moonshot
 * @param modelId (optional) — Model identifier or preset slug (e.g., moonshot-v1-8k)
 * @returns model — Structured Bit describing the Moonshot provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildMoonshot({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Builds a model via the Mozilla any-llm gateway (OpenAI-compatible). Supports both self-hosted gateways and the managed platform at any-llm.ai
 * @param endpoint (optional) — Mozilla any-llm gateway base URL (e.g. http://localhost:8000/v1 for self-hosted or https://api.any-llm.ai/v1 for managed platform)
 * @param apiKey (optional) — API key for authenticating against the any-llm gateway or platform
 * @param modelId (optional) — Model identifier in provider:model format (e.g. openai:gpt-4o, anthropic:claude-sonnet-4-20250514)
 * @returns model — Structured Bit describing the Mozilla any-llm provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildMozilla({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Builds the Ollama model based on certain selection criteria
 * @param endpoint (optional) — Local or remote Ollama HTTP endpoint
 * @param modelId (optional) — Model identifier/tag to run (must exist on the Ollama host)
 * @returns model — Structured Bit describing the Ollama provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildOllama({ endpoint?: string, modelId?: string }): Struct;

/**
 * Prepares a Bit for OpenAI or Azure OpenAI endpoints with the provided credentials
 * @param provider (optional) — Choose OpenAI cloud or Azure OpenAI
 * @param endpoint (optional) — Base API endpoint (override for Azure or proxies)
 * @param apiKey (optional) — API key or Azure key used for authentication
 * @returns model — Bit containing the provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildOpenai({ provider?: string, endpoint?: string, apiKey?: string }): Struct;

/**
 * Builds the OpenRouter model based on certain selection criteria
 * @param endpoint (optional) — OpenRouter base URL or regional proxy
 * @param apiKey (optional) — Token used for authenticating against OpenRouter
 * @param modelId (optional) — Model identifier from OpenRouter's catalog
 * @returns model — Structured Bit describing the OpenRouter provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildOpenrouter({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Builds the Perplexity model based on certain selection criteria
 * @param endpoint (optional) — Perplexity API endpoint or self-hosted base URL
 * @param apiKey (optional) — Token used for authenticating against Perplexity
 * @param modelId (optional) — Model identifier or preset slug to request
 * @returns model — Structured Bit describing the Perplexity provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildPerplexity({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Builds the Together AI model based on certain selection criteria
 * @param endpoint (optional) — Together API endpoint or regional proxy
 * @param apiKey (optional) — Token used for authenticating against Together
 * @param modelId (optional) — Model identifier or preset slug to request
 * @returns model — Structured Bit describing the Together provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildTogether({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Prepares a Bit for Google Vertex AI Gemini endpoints using ADC or service account credentials
 * @param projectId (optional) — Google Cloud project ID. Leave empty to use GOOGLE_CLOUD_PROJECT or the service account project_id.
 * @param location (optional) — Vertex AI location
 * @param serviceAccountJson (optional) — Optional Google Cloud service account key JSON. Leave empty to use Application Default Credentials.
 * @param accessToken (optional) — Optional OAuth access token. Prefer ADC or a service account for long-running flows.
 * @param modelId (optional) — Vertex AI Gemini model identifier
 * @returns model — Bit containing the provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildVertex({ projectId?: string, location?: string, serviceAccountJson?: string, accessToken?: string, modelId?: string }): Struct;

/**
 * Builds the VoyageAI model based on certain selection criteria
 * @param endpoint (optional) — VoyageAI API base URL or custom proxy
 * @param apiKey (optional) — Token used for authenticating against VoyageAI
 * @param modelId (optional) — Model identifier or preset slug to use
 * @returns model — Structured Bit describing the VoyageAI provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildVoyageai({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

/**
 * Builds the xAI model based on certain selection criteria
 * @param endpoint (optional) — xAI API endpoint or custom proxy
 * @param apiKey (optional) — Token used for authenticating against xAI
 * @param modelId (optional) — Model identifier or preset slug to request (e.g., grok-2-1212)
 * @returns model — Structured Bit describing the xAI provider
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildXai({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;


// === AI/Generative/Response ===

/**
 * Wraps an arbitrary string in a synthetic streaming chunk
 * @param content (optional) — Plain text that should stream to clients
 * @returns chunk — Response chunk built from the provided text
 */
declare function aiGenerativeLlmChunkFromString({ content?: string }): Struct;

/**
 * Wraps a plain string into a synthetic LLM response object for downstream tooling.
 * @param content (optional) — Plain assistant text that should be wrapped into a Response object.
 * @returns response — LLM-style Response struct containing the provided content as a single assistant message.
 */
declare function aiGenerativeLlmResponseFromString({ content?: string }): Struct;

/**
 * Extracts the content string from the last assistant message in a response
 * @param response — LLM response to extract from
 * @returns content — Content string from the last message
 * @returns success — Whether content was successfully extracted
 * @returns parts — Ordered text and media content parts
 * @returns images — Image URLs or data URIs
 * @returns audio — Audio URLs or data URIs
 * @returns videos — Video URLs or data URIs
 * @returns documents — Document URLs or data URIs
 * @returns reasoning — Displayable reasoning returned by the model
 */
declare function aiGenerativeLlmResponseLastContent({ response: Struct }): { content: string, success: bool, parts: Struct[], images: string[], audio: string[], videos: string[], documents: string[], reasoning: string };

/**
 * Extracts the last assistant message from a response
 * @param response — LLM response to inspect
 * @returns message — Last message from the response
 * @returns success — Whether a message was successfully extracted
 */
declare function aiGenerativeLlmResponseLastMessage({ response: Struct }): { message: Struct, success: bool };

/**
 * Creates an empty Response struct for manual composition
 * @returns response — Empty Response ready to populate
 */
declare function aiGenerativeLlmResponseMake(): Struct;

/**
 * Appends a streaming chunk onto a response
 * @param response — Response object that should receive the chunk
 * @param chunk — Chunk to append
 * @returns responseOut — Response including the appended chunk
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeLlmResponsePushChunk({ response: Struct, chunk: Struct }): Struct;


// === AI/Generative/Response/Chunk ===

/**
 * Extracts the latest streamed token from a response chunk
 * @param chunk — Response chunk that carries streamed tokens
 * @returns token — Most recent streamed token
 */
declare function aiGenerativeLlmResponseChunkGetToken({ chunk: Struct }): string;


// === AI/Generative/Response/Message ===

/**
 * Extracts the text content field from a response message
 * @param message — Message to extract content from
 * @returns content — Content string from the message
 * @returns success — Whether content was successfully extracted
 * @returns parts — Ordered text and media content parts
 * @returns images — Image URLs or data URIs
 * @returns audio — Audio URLs or data URIs
 * @returns videos — Video URLs or data URIs
 * @returns documents — Document URLs or data URIs
 * @returns reasoning — Displayable reasoning returned by the model
 */
declare function aiGenerativeLlmResponseMessageGetContent({ message: Struct }): { content: string, success: bool, parts: Struct[], images: string[], audio: string[], videos: string[], documents: string[], reasoning: string };

/**
 * Extracts the author role string from a response message
 * @param message — Message to extract the role from
 * @returns role — Role string from the message
 */
declare function aiGenerativeLlmResponseMessageGetRole({ message: Struct }): string;


// === AI/Generative/Video ===

/**
 * Generates video with an existing provider Bit and writes it to FlowPath.
 * @param provider — Existing provider Bit
 * @param prompt (optional) — Video prompt
 * @param outputPath — Destination FlowPath for generated video
 * @param firstFrame — Optional image FlowPath for image-to-video
 * @param lastFrame — Optional ending image FlowPath for providers that support it
 * @param inputVideo — Optional source video FlowPath for video-to-video or extension
 * @param providerOptions (optional) — Typed provider-specific video options
 * @returns path — First generated video path
 * @returns paths — Generated video paths
 * @returns metadata — Generation metadata
 * @impure has side effects / drives control flow
 */
declare function aiVideoGenerate({ provider: Struct, prompt?: string, outputPath: Struct, firstFrame: Struct, lastFrame: Struct, inputVideo: Struct, providerOptions?: Struct }): { path: Struct, paths: Struct[], metadata: Struct };


// === AI/Generative/Video/Options ===

/**
 * Creates typed video options for fal.ai video models.
 * @param negativePrompt (optional) — Text describing what to avoid
 * @param aspectRatio (optional) — fal aspect ratio
 * @param size (optional) — fal output resolution
 * @param durationSeconds (optional) — Requested duration in seconds. Use 0 for provider default.
 * @param seed (optional) — Optional deterministic seed. Use 0 for provider default.
 * @param generateAudio (optional) — Generate native audio when the provider supports it
 * @param pollIntervalSeconds (optional) — Seconds between provider status checks
 * @param maxWaitSeconds (optional) — Maximum seconds to wait for completion
 * @returns options — Typed video generation provider options
 */
declare function aiVideoOptionsFal({ negativePrompt?: string, aspectRatio?: string, size?: string, durationSeconds?: int, seed?: int, generateAudio?: bool, pollIntervalSeconds?: int, maxWaitSeconds?: int }): Struct;

/**
 * Creates typed video options for OpenAI Sora models.
 * @param size (optional) — Sora video size
 * @param durationSeconds (optional) — Requested duration in seconds. Use 0 for provider default.
 * @param pollIntervalSeconds (optional) — Seconds between provider status checks
 * @param maxWaitSeconds (optional) — Maximum seconds to wait for completion
 * @returns options — Typed video generation provider options
 */
declare function aiVideoOptionsOpenaiSora({ size?: string, durationSeconds?: int, pollIntervalSeconds?: int, maxWaitSeconds?: int }): Struct;

/**
 * Creates typed video options for Replicate video models.
 * @param negativePrompt (optional) — Text describing what to avoid
 * @param aspectRatio (optional) — Replicate aspect ratio
 * @param size (optional) — Replicate output resolution
 * @param durationSeconds (optional) — Requested duration in seconds. Use 0 for provider default.
 * @param seed (optional) — Optional deterministic seed. Use 0 for provider default.
 * @param generateAudio (optional) — Generate native audio when the provider supports it
 * @param pollIntervalSeconds (optional) — Seconds between provider status checks
 * @param maxWaitSeconds (optional) — Maximum seconds to wait for completion
 * @returns options — Typed video generation provider options
 */
declare function aiVideoOptionsReplicate({ negativePrompt?: string, aspectRatio?: string, size?: string, durationSeconds?: int, seed?: int, generateAudio?: bool, pollIntervalSeconds?: int, maxWaitSeconds?: int }): Struct;

/**
 * Creates typed video options for Runway models.
 * @param aspectRatio (optional) — Runway aspect ratio
 * @param size (optional) — Runway output size
 * @param durationSeconds (optional) — Requested duration in seconds. Use 0 for provider default.
 * @param seed (optional) — Optional deterministic seed. Use 0 for provider default.
 * @param pollIntervalSeconds (optional) — Seconds between provider status checks
 * @param maxWaitSeconds (optional) — Maximum seconds to wait for completion
 * @returns options — Typed video generation provider options
 */
declare function aiVideoOptionsRunway({ aspectRatio?: string, size?: string, durationSeconds?: int, seed?: int, pollIntervalSeconds?: int, maxWaitSeconds?: int }): Struct;

/**
 * Creates typed video options for Google Vertex Veo models.
 * @param negativePrompt (optional) — Text describing what to avoid
 * @param aspectRatio (optional) — Veo aspect ratio
 * @param size (optional) — Veo output resolution
 * @param durationSeconds (optional) — Requested duration in seconds. Use 0 for provider default.
 * @param seed (optional) — Optional deterministic seed. Use 0 for provider default.
 * @param count (optional) — Number of videos to request
 * @param pollIntervalSeconds (optional) — Seconds between provider status checks
 * @param maxWaitSeconds (optional) — Maximum seconds to wait for completion
 * @returns options — Typed video generation provider options
 */
declare function aiVideoOptionsVertexVeo({ negativePrompt?: string, aspectRatio?: string, size?: string, durationSeconds?: int, seed?: int, count?: int, pollIntervalSeconds?: int, maxWaitSeconds?: int }): Struct;


// === AI/Generative/Video/Provider ===

/**
 * Builds a fal.ai queued video generation provider Bit.
 * @param apiKey (optional) — fal API key
 * @param endpoint (optional) — fal queue endpoint
 * @param modelId (optional) — fal model path
 * @returns provider — Bit containing the video generation provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiVideoBuildFal({ apiKey?: string, endpoint?: string, modelId?: string }): Struct;

/**
 * Builds a Replicate video generation provider Bit.
 * @param apiKey (optional) — Replicate API token
 * @param endpoint (optional) — Replicate API endpoint
 * @param modelId (optional) — Replicate owner/model path for official models
 * @param version (optional) — Optional model version hash for community predictions
 * @returns provider — Bit containing the video generation provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiVideoBuildReplicate({ apiKey?: string, endpoint?: string, modelId?: string, version?: string }): Struct;

/**
 * Builds a Runway video generation provider Bit.
 * @param apiKey (optional) — Runway API key
 * @param endpoint (optional) — Runway API endpoint
 * @param apiVersion (optional) — Runway API version header
 * @param modelId (optional) — Runway video model ID
 * @returns provider — Bit containing the video generation provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiVideoBuildRunway({ apiKey?: string, endpoint?: string, apiVersion?: string, modelId?: string }): Struct;


// === AI/GitHub/Copilot/Chat ===

/**
 * Aborts the current message processing
 * @param session — Copilot session
 * @impure has side effects / drives control flow
 */
declare function copilotAbort({ session: Struct }): void;

/**
 * Sends a message to Copilot and waits for complete response. Supports history input for context.
 * @param session — Copilot session
 * @param prompt — Message to send
 * @param history — Optional chat history for context (same format as Model Invoke)
 * @returns response — Complete response text
 * @returns result — Response in standard model format (matches Model Invoke)
 * @returns stats — Token usage, cost, and model statistics
 * @impure has side effects / drives control flow
 */
declare function copilotSendAndWait({ session: Struct, prompt: string, history: Struct }): { response: string, result: Struct, stats: Struct };

/**
 * Sends a message to Copilot and streams the response. Supports history input and matches Model Invoke interface.
 * @param session — Copilot session
 * @param prompt — Message to send
 * @param history — Optional chat history for context (same format as Model Invoke)
 * @returns chunk — Current streaming chunk (matches Model Invoke ResponseChunk format)
 * @returns result — Complete response (matches Model Invoke Response format)
 * @returns fullResponse — Complete accumulated response text
 * @returns stats — Token usage, cost, and model statistics
 * @impure has side effects / drives control flow
 */
declare function copilotSendStreaming({ session: Struct, prompt: string, history: Struct }): { chunk: Struct, result: Struct, fullResponse: string, stats: Struct };


// === AI/GitHub/Copilot/Client ===

/**
 * Gracefully stops a running Copilot client (local or server)
 * @param client — Client handle to stop
 * @impure has side effects / drives control flow
 */
declare function copilotClientStop({ client: Struct }): void;

/**
 * Builds a local Copilot client configuration (stdio-based). Requires 'copilot' CLI to be installed and in PATH, or specify the CLI path explicitly.
 * @param logLevel (optional) — Client log level
 * @param cliPath (optional) — Optional path to Copilot CLI executable. If not set, searches PATH and COPILOT_CLI_PATH env var.
 * @returns clientConfig — Local client configuration
 */
declare function copilotLocalClientBuilder({ logLevel?: string, cliPath?: string }): Struct;

/**
 * Starts a local Copilot client using stdio. Requires 'copilot' CLI installed.
 * @param clientConfig — Local client configuration
 * @returns client — Running client handle
 * @returns errorMessage — Error message if startup fails
 * @impure has side effects / drives control flow
 */
declare function copilotLocalClientStart({ clientConfig: Struct }): { client: Struct, errorMessage: string };

/**
 * Builds a server/remote Copilot client configuration (TCP-based)
 * @param url — TCP endpoint URL (e.g., tcp://localhost:3000)
 * @param logLevel (optional) — Client log level
 * @returns clientConfig — Server client configuration
 */
declare function copilotServerClientBuilder({ url: string, logLevel?: string }): Struct;

/**
 * Starts a server/remote Copilot client using TCP
 * @param clientConfig — Server client configuration
 * @returns client — Running client handle
 * @returns errorMessage — Error message if connection fails
 * @impure has side effects / drives control flow
 */
declare function copilotServerClientStart({ clientConfig: Struct }): { client: Struct, errorMessage: string };


// === AI/GitHub/Copilot/Config ===

/**
 * Configures a custom agent
 * @param name — Agent identifier
 * @param displayName (optional) — Human-readable agent name
 * @param description (optional) — Agent description
 * @param prompt — Agent system prompt
 * @returns agent — Custom agent configuration
 */
declare function copilotCustomAgent({ name: string, displayName?: string, description?: string, prompt: string }): Struct;

/**
 * Configures infinite session with automatic context compaction
 * @param enabled (optional) — Enable infinite sessions
 * @param backgroundThreshold (optional) — Background compaction threshold (0.0-1.0)
 * @param exhaustionThreshold (optional) — Buffer exhaustion threshold (0.0-1.0)
 * @returns config — Infinite session configuration
 */
declare function copilotInfiniteSession({ enabled?: bool, backgroundThreshold?: float, exhaustionThreshold?: float }): Struct;

/**
 * Configures a custom provider (Bring Your Own Key)
 * @param baseUrl — Provider API base URL (e.g., https://api.openai.com/v1)
 * @param apiKey — API key for authentication
 * @param model (optional) — Model ID to use
 * @returns config — Provider configuration
 */
declare function copilotProviderConfig({ baseUrl: string, apiKey: string, model?: string }): Struct;

/**
 * Configures the system message for the session
 * @param content — System message content
 * @param mode (optional) — Replace or Append to default system message
 * @returns config — System message configuration
 */
declare function copilotSystemMessage({ content: string, mode?: string }): Struct;


// === AI/GitHub/Copilot/MCP ===

/**
 * Configures an HTTP/SSE MCP server for remote tool integration
 * @param url — HTTP endpoint URL
 * @param tools (optional) — Tool filter (use ["*"] for all tools)
 * @param timeout (optional) — Server timeout in milliseconds
 * @returns config — MCP server configuration
 */
declare function copilotMcpHttpServer({ url: string, tools?: string[], timeout?: int }): Struct;

/**
 * Configures a local/stdio MCP server for tool integration
 * @param command — Command to execute (e.g., npx, python)
 * @param args (optional) — Command arguments
 * @param tools (optional) — Tool filter (use ["*"] for all tools)
 * @param timeout (optional) — Server timeout in milliseconds
 * @returns config — MCP server configuration
 */
declare function copilotMcpLocalServer({ command: string, args?: string[], tools?: string[], timeout?: int }): Struct;


// === AI/GitHub/Copilot/Session ===

/**
 * Creates a new Copilot chat session
 * @param client — Running Copilot client
 * @param config — Session configuration (from Session Builder)
 * @returns session — Session handle
 * @impure has side effects / drives control flow
 */
declare function copilotCreateSession({ client: Struct, config: Struct }): Struct;

/**
 * Destroys a Copilot session
 * @param session — Session handle to destroy
 * @impure has side effects / drives control flow
 */
declare function copilotDestroySession({ session: Struct }): void;

/**
 * Builds a complete Copilot session configuration with all options
 * @param model (optional) — Optional model ID to use (e.g., gpt-4o)
 * @param streaming (optional) — Enable streaming responses
 * @param systemMessage (optional) — Optional system message content
 * @param systemMode (optional) — Replace or Append to default system message
 * @param infiniteEnabled (optional) — Enable infinite sessions with automatic context compaction
 * @param backgroundThreshold (optional) — Background compaction threshold (0.0-1.0)
 * @param exhaustionThreshold (optional) — Buffer exhaustion threshold (0.0-1.0)
 * @param provider — Optional BYOK provider configuration
 * @param tools — Optional array of tool configurations
 * @param customAgents — Optional array of custom agent configurations
 * @param mcpServers — Optional MCP servers configuration (JSON object)
 * @returns config — Complete session configuration
 */
declare function copilotSessionBuilder({ model?: string, streaming?: bool, systemMessage?: string, systemMode?: string, infiniteEnabled?: bool, backgroundThreshold?: float, exhaustionThreshold?: float, provider: Struct, tools: Struct[], customAgents: Struct[], mcpServers: any }): Struct;


// === AI/GitHub/Copilot/Tools ===

/**
 * Configures an agent tool with parameters
 * @param name — Tool name
 * @param description — Tool description
 * @param schema (optional) — Tool parameters JSON schema
 * @returns tool — Configured tool
 */
declare function copilotToolConfig({ name: string, description: string, schema?: Struct }): Struct;

/**
 * Combines multiple tools into a list for session configuration
 * @param tool1 — Optional tool configuration
 * @param tool2 — Optional tool configuration
 * @param tool3 — Optional tool configuration
 * @param tool4 — Optional tool configuration
 * @param tool5 — Optional tool configuration
 * @param tool6 — Optional tool configuration
 * @param tool7 — Optional tool configuration
 * @param tool8 — Optional tool configuration
 * @returns tools — List of configured tools
 */
declare function copilotToolList({ tool1: Struct, tool2: Struct, tool3: Struct, tool4: Struct, tool5: Struct, tool6: Struct, tool7: Struct, tool8: Struct }): Struct[];


// === AI/GitHub/Copilot/Utilities ===

/**
 * Checks if a Copilot client is connected and ready
 * @param client — Copilot client handle
 * @returns isConnected — Whether the client is connected
 * @returns clientId — Client identifier
 */
declare function copilotClientStatus({ client: Struct }): { isConnected: bool, clientId: string };

/**
 * Gets the authentication status of the Copilot client
 * @param client — Copilot client handle
 * @returns isAuthenticated — Whether the user is authenticated
 * @returns login — GitHub username if authenticated
 */
declare function copilotGetAuthStatus({ client: Struct }): { isAuthenticated: bool, login: string };

/**
 * Lists available Copilot models
 * @param client — Copilot client handle
 * @returns models — Array of available model names
 */
declare function copilotGetModels({ client: Struct }): string[];

/**
 * Gets the version of the Copilot CLI
 * @param client — Copilot client handle
 * @returns version — CLI version string
 */
declare function copilotGetVersion({ client: Struct }): string;


// === AI/ML ===

/**
 * Extract class_idx and label from predictions.
 * @param prediction — Single ClassPrediction
 * @returns classIdx — Selected prediction class index
 * @returns label — Selected prediction label (empty if not provided)
 */
declare function aiMlPredClassOrLabel({ prediction: Struct }): { classIdx: int, label: string };

/**
 * Image classification using Teachable Machine models.
 * @param model — Path to *.tflite model
 * @param imageIn — Image Object
 * @param labels — Optional labels.txt
 * @param inputWidth (optional) — Model input width
 * @param inputHeight (optional) — Model input height
 * @returns predictions — Class Predictions
 * @impure has side effects / drives control flow
 */
declare function aiMlTeachableMachine({ model: Struct, imageIn: Struct, labels: Struct, inputWidth?: int, inputHeight?: int }): Struct[];

/**
 * Load Trained ML Model from Path
 * @param path — Filesystem or storage path pointing at the serialized model JSON
 * @returns model — Handle to the loaded machine learning model
 * @impure has side effects / drives control flow
 */
declare function loadMlModel({ path: Struct }): Struct;

/**
 * Load Trained ML Model from Path using fast binary format (Fory)
 * @param path — Filesystem or storage path pointing at the serialized model binary (.flmodel)
 * @returns model — Handle to the loaded machine learning model
 * @impure has side effects / drives control flow
 */
declare function loadMlModelBinary({ path: Struct }): Struct;

/**
 * Predict with Machine Learning Model
 * @param model — Trained ML model to use for inference
 * @param source (optional) — Choose the input type for prediction (database rows or raw vector)
 * @param batchSize (optional) — Number of records to process per batch (default: 5000, 0 = process all at once)
 * @impure has side effects / drives control flow
 */
declare function mlPredict({ model: Struct, source?: string, batchSize?: int }): void;

/**
 * Save Trained ML Model to Path
 * @param model — Any trained ML model handle to persist
 * @param path — Destination path where the model JSON should be written
 * @impure has side effects / drives control flow
 */
declare function saveMlModel({ model: Struct, path: Struct }): void;

/**
 * Save Trained ML Model to Path using fast binary format (Fory)
 * @param model — Any trained ML model handle to persist
 * @param path — Destination path where the model binary should be written (.flmodel)
 * @impure has side effects / drives control flow
 */
declare function saveMlModelBinary({ model: Struct, path: Struct }): void;


// === AI/ML/Classification ===

/**
 * Fit/Train an AdaBoost classifier using multi-class SAMME boosting over shallow Decision Trees. Each learner focuses on the rows its predecessors got wrong, so boosting usually beats a single tree on weak signal, but it is far more sensitive to label noise and outliers than Random Forest. Estimators is a maximum, not a guarantee: boosting stops early once a learner is no better than random guessing.
 * @param source (optional) — Choose which backend supplies the training data
 * @param nEstimators (optional) — Maximum number of boosting rounds. Boosting stops early once a learner performs no better than random guessing, so the fitted model may hold fewer estimators than requested.
 * @param learningRate (optional) — Shrinkage applied to each learner's vote. Must be positive. Values below 1 regularize the ensemble but need more estimators; 0.1 with 500 estimators is a common pairing.
 * @param maxDepth (optional) — Depth of each weak learner. AdaBoost is designed around shallow trees; 1 gives classic decision stumps. Deep base trees defeat the point of boosting and overfit quickly.
 * @param seed (optional) — Seed for the base learner sampling. Fixing it makes the sampling reproducible. Note that the base trees are not bit-exact across processes: linfa resolves modal-class ties in hash-map iteration order, which Rust re-randomizes on every run.
 * @returns model — Thread-safe handle to the trained AdaBoost classifier
 * @returns estimatorsKept — Number of estimators actually retained after early stopping, which may be lower than the requested maximum
 * @impure has side effects / drives control flow
 */
declare function fitAdaboost({ source?: string, nEstimators?: int, learningRate?: float, maxDepth?: int, seed?: int }): { model: Struct, estimatorsKept: int };

/**
 * Fit/Train a Decision Tree classifier. Native multi-class support with interpretable rules.
 * @param source (optional) — Choose which backend supplies the training data
 * @param maxDepth (optional) — Maximum depth of the tree. None means unlimited.
 * @param minSamplesSplit (optional) — Minimum number of samples required to split a node
 * @param splitQuality (optional) — Impurity metric that scores candidate splits. Gini is cheaper, Entropy favours balanced information gain.
 * @param minWeightLeaf (optional) — Minimum number of samples (total sample weight) a split has to place in each leaf
 * @param minImpurityDecrease (optional) — Minimum impurity decrease a split has to bring to be applied. Must be greater than zero; larger values prune harder.
 * @returns model — Thread-safe handle to the trained Decision Tree classifier
 * @impure has side effects / drives control flow
 */
declare function fitDecisionTree({ source?: string, maxDepth?: int, minSamplesSplit?: int, splitQuality?: string, minWeightLeaf?: float, minImpurityDecrease?: float }): Struct;

/**
 * Fit a K-Nearest-Neighbours classifier. Non-parametric and instance based: the fitted model embeds a verbatim copy of the whole training set instead of learned coefficients, so every training row (and any personal data in it) travels with the model, is written into every saved model file and can be reconstructed by anyone holding it. Treat the model with the same care as the source table.
 * @param source (optional) — Choose which backend supplies the training data
 * @param k (optional) — How many nearest training rows vote on each prediction. Must be at least 1 and cannot exceed the number of training rows. Larger values smooth the decision boundary.
 * @param distanceWeighted (optional) — Weight each neighbour by the inverse of its distance instead of counting every neighbour equally. Helps when k is large or classes overlap.
 * @returns model — Thread-safe handle to the trained KNN classifier. Contains a full copy of the training set.
 * @impure has side effects / drives control flow
 */
declare function fitKnnClassifier({ source?: string, k?: int, distanceWeighted?: bool }): Struct;

/**
 * Fit/Train a Logistic Regression classifier with L2 regularization. Handles binary and multi-class targets and yields interpretable coefficients plus calibrated probabilities. The solver expects features on a comparable scale - fit a Feature Scaler first if your columns have very different ranges.
 * @param source (optional) — Choose which backend supplies the training data
 * @param mode (optional) — Auto picks the binary solver for two classes and the multinomial (softmax) solver for more. Binary and Multinomial force one of them.
 * @param alpha (optional) — Weight of the L2 penalty on the coefficients. 0 disables regularization, larger values shrink the model harder.
 * @param fitIntercept (optional) — Fit a bias term. Disable only when the features are already centered.
 * @param maxIterations (optional) — Upper bound on LBFGS iterations. Raise it when training accuracy stays at the baseline.
 * @param gradientTolerance (optional) — Smallest gradient norm that still continues the solver. Smaller means a tighter fit and more iterations.
 * @param threshold (optional) — Probability above which linfa's positive class is predicted. You do not choose that class: linfa assigns it to whichever label sorts second, which for a typical imbalanced dataset is the majority class. Raising the threshold therefore makes the OTHER class — usually the rare one — more likely to be predicted. The class the threshold actually governs is logged when training runs. Binary mode only, ignored for multinomial targets.
 * @returns model — Thread-safe handle to the trained Logistic Regression classifier
 * @impure has side effects / drives control flow
 */
declare function fitLogisticRegression({ source?: string, mode?: string, alpha?: float, fitIntercept?: bool, maxIterations?: int, gradientTolerance?: float, threshold?: float }): Struct;

/**
 * Fit/Train a Multinomial Naive Bayes classifier, the standard baseline for text and other count data. Features must be non-negative counts or TF-IDF weights, which is what the Fit TF-IDF Vectorizer node produces. Native multi-class support and a single pass over the data.
 * @param source (optional) — Choose which backend supplies the training data
 * @param alpha (optional) — Additive (Laplace/Lidstone) smoothing added to every feature count. 1.0 is the usual choice; smaller values trust the training counts more, and 0 disables smoothing so any term unseen in a class makes that class impossible.
 * @returns model — Thread-safe handle to the trained Multinomial Naive Bayes classifier
 * @impure has side effects / drives control flow
 */
declare function fitMultinomialNaiveBayes({ source?: string, alpha?: float }): Struct;

/**
 * Fit/Train a Gaussian Naive Bayes classifier. Native multi-class support - no need for One-vs-All.
 * @param source (optional) — Choose which backend supplies the training data
 * @returns model — Thread-safe handle to the trained Naive Bayes classifier
 * @impure has side effects / drives control flow
 */
declare function fitNaiveBayes({ source?: string }): Struct;

/**
 * Fit a One-Class SVM on normal observations only. Predictions flag whether a new row is an inlier (1) or an outlier (0).
 * @param source (optional) — Choose which backend supplies the training data
 * @param nu (optional) — Upper bound on the fraction of training rows the model is allowed to treat as outliers, in (0, 1]. Raise it when the training set is known to be contaminated.
 * @param kernel (optional) — Feature-space mapping. Gaussian wraps a tight non-linear boundary around the data, Linear yields a half-space, Polynomial adds interaction terms.
 * @param kernelParam (optional) — Gaussian: the eps in exp(-||x - x'||^2 / eps), larger means a looser boundary. Polynomial: the degree of (<x, x'> + 1)^degree. Ignored for Linear.
 * @param tolerance (optional) — Stopping threshold of the SMO solver. Smaller values train longer for a more precise boundary.
 * @returns model — Thread-safe handle to the trained One-Class SVM
 * @returns supportVectors — Number of training rows that define the learned boundary
 * @impure has side effects / drives control flow
 */
declare function fitOneClassSvm({ source?: string, nu?: float, kernel?: string, kernelParam?: float, tolerance?: float }): { model: Struct, supportVectors: int };

/**
 * Fit/Train a Random Forest classifier: many Decision Trees, each grown on a bootstrapped sample of the rows and a random subset of the features, combined by majority vote. Far more robust to overfitting than a single tree, at the price of interpretability. Model size and fit time grow linearly with Ensemble Size, so a forest of 500 trees costs roughly 500x a single tree.
 * @param source (optional) — Choose which backend supplies the training data
 * @param ensembleSize (optional) — Number of Decision Trees to grow. Both fit time and the size of the saved model scale linearly with this value.
 * @param bootstrapProportion (optional) — Share of the training rows drawn (with replacement) for each tree. Must be greater than 0 and at most 1.
 * @param featureProportion (optional) — Share of the features offered to each tree. Must be at most 1. Leave at 0 for the textbook default of sqrt(feature count) features per tree.
 * @param maxDepth (optional) — Maximum depth of each tree. 0 or less means unlimited, which grows deeper trees and a larger model.
 * @param minWeightSplit (optional) — Minimum summed sample weight a node needs before it may be split. Without row weights this is simply the minimum number of samples.
 * @param seed (optional) — Seed for the bootstrap and feature sampling. Fixing it makes the row and feature draws reproducible. Note that the base trees are not bit-exact across processes: linfa resolves modal-class ties in hash-map iteration order, which Rust re-randomizes on every run.
 * @returns model — Thread-safe handle to the trained Random Forest classifier
 * @impure has side effects / drives control flow
 */
declare function fitRandomForest({ source?: string, ensembleSize?: int, bootstrapProportion?: float, featureProportion?: float, maxDepth?: int, minWeightSplit?: float, seed?: int }): Struct;

/**
 * Fit/Train Support Vector Machines (SVM) for Multi-Class Classification
 * @param source (optional) — Choose which backend supplies the training data
 * @param kernel (optional) — Feature-space mapping. Gaussian separates non-linear classes, Linear is the plain SVM, Polynomial adds interaction terms.
 * @param kernelParam (optional) — Gaussian: the eps in exp(-||x - x'||^2 / eps), larger means smoother boundaries. Polynomial: the degree of (<x, x'> + 1)^degree. Ignored for Linear.
 * @param c (optional) — Penalty for misclassified training rows, applied to both the positive and the negative side. Higher values fit the training data harder and risk overfitting.
 * @returns model — Thread-safe handle to the trained SVM classifier
 * @impure has side effects / drives control flow
 */
declare function fitSvmMultiClass({ source?: string, kernel?: string, kernelParam?: float, c?: float }): Struct;


// === AI/ML/Clustering ===

/**
 * Fit/Train DBSCAN Density-Based Clustering
 * @param epsilon (optional) — Maximum distance between points in the same cluster
 * @param minPoints (optional) — Minimum points required to form a dense region
 * @param source (optional) — Choose which backend supplies the training data
 * @returns nClusters — Number of clusters found (excluding noise)
 * @returns nNoise — Number of points classified as noise
 * @impure has side effects / drives control flow
 */
declare function fitDbscan({ epsilon?: float, minPoints?: int, source?: string }): { nClusters: int, nNoise: int };

/**
 * Fit/Train a Gaussian Mixture Model. Soft clustering with per-component covariances and mixture weights, fitted by Expectation-Maximization.
 * @param source (optional) — Choose which backend supplies the training data
 * @param nClusters (optional) — Number of Gaussian components (k) in the mixture. Each component costs a full d x d covariance matrix.
 * @param covarianceType (optional) — Shape of each component's covariance. linfa 0.8 implements full covariances only - scikit-learn's diag, tied and spherical variants do not exist here, so every component always costs d x d parameters.
 * @param initMethod (optional) — How initial responsibilities are built: KMeans runs a KMeans pass first (usually the better optimum), Random draws them uniformly.
 * @param nRuns (optional) — Number of EM passes. Note: linfa 0.8 continues each pass from the previous parameters instead of re-initializing, so this multiplies the iteration budget (Runs x Max Iterations) rather than performing independent restarts. Vary the Seed for a genuinely different start.
 * @param tolerance (optional) — EM stops once the average log-likelihood gain per iteration falls below this value
 * @param regCovariance (optional) — Non-negative value added to each covariance diagonal to keep it positive definite. Raise it when the fit reports a singular covariance; 0 makes duplicate or constant rows fail outright.
 * @param maxNIterations (optional) — Maximum number of EM iterations per run
 * @param seed (optional) — Seed for the training row order. linfa 0.8 hard-codes its internal RNG (seed 42) and exposes no seeding hook on this entry point, so changing the seed re-orders the rows, which is what changes the initial responsibilities. Keep 42 to reproduce linfa's stock ordering.
 * @returns model — Thread-safe handle to the trained Gaussian Mixture model
 * @returns weights — Fitted mixture proportions, one per component, summing to 1. A tiny weight means that component captured almost no data.
 * @impure has side effects / drives control flow
 */
declare function fitGaussianMixture({ source?: string, nClusters?: int, covarianceType?: string, initMethod?: string, nRuns?: int, tolerance?: float, regCovariance?: float, maxNIterations?: int, seed?: int }): { model: Struct, weights: float[] };

/**
 * Fit/Train KMeans Clustering
 * @param cluster (optional) — Choose how many centroids to fit
 * @param source (optional) — Choose which backend supplies the training data
 * @returns model — Thread-safe handle to the trained KMeans model
 * @impure has side effects / drives control flow
 */
declare function fitKmeans({ cluster?: int, source?: string }): Struct;


// === AI/ML/Dataset ===

/**
 * Generate K train/test splits for cross-validation. Each fold uses (K-1)/K data for training and 1/K for validation, and runs the connected fold branch once per fold.
 * @param k (optional) — Number of folds for cross-validation (typically 5 or 10)
 * @param shuffle (optional) — Randomly shuffle data before splitting
 * @param source — Source database containing the dataset. It is only read, never modified.
 * @param trainDb — Database to receive training data for each fold (will be cleared and filled K times)
 * @param testDb — Database to receive validation data for each fold (will be cleared and filled K times)
 * @returns foldIndex — Current fold index (0 to K-1)
 * @returns info — Information about the K-fold split
 * @impure has side effects / drives control flow
 */
declare function aiMlDatasetKfold({ k?: int, shuffle?: bool, source: Struct, trainDb: Struct, testDb: Struct }): { foldIndex: int, info: Struct };

/**
 * Random sample N records or a ratio from a dataset
 * @param sampleCount (optional) — Number of records to sample (if set, takes precedence over ratio)
 * @param sampleRatio (optional) — Ratio of records to sample (0.0 to 1.0, used if sample_count is 0)
 * @param source — Data Source (DB or CSV)
 * @param target — Destination database connection that receives the sampled rows
 * @returns sampledCount — Number of records that were sampled
 * @impure has side effects / drives control flow
 */
declare function aiMlDatasetSample({ sampleCount?: int, sampleRatio?: float, source: Struct, target: Struct }): int;

/**
 * Shuffle dataset rows randomly
 * @param source — Data Source (DB or CSV)
 * @param target — Destination database connection that receives the shuffled rows
 * @impure has side effects / drives control flow
 */
declare function aiMlDatasetShuffle({ source: Struct, target: Struct }): void;

/**
 * Split a dataset into training and testing subsets
 * @param split (optional) — Ratio used for assigning rows to the training set (rest goes to test)
 * @param source — Data Source (DB or CSV)
 * @param train — Destination database connection that receives the training rows
 * @param test — Destination database connection that receives the testing rows
 * @impure has side effects / drives control flow
 */
declare function aiMlDatasetSplit({ split?: float, source: Struct, train: Struct, test: Struct }): void;

/**
 * Split a dataset into training and testing subsets, keeping every class at its original proportion in both subsets
 * @param split (optional) — Share of each class that goes to the training set (rest goes to test). Must be between 0 and 1, exclusive
 * @param labelColumn (optional) — Name of the column containing class labels for stratification
 * @param seed (optional) — Seed for the per-class shuffle. Any non-zero value makes the split reproducible; 0 draws a fresh seed each run and logs it
 * @param source — Data Source (DB or CSV)
 * @param train — Destination database that receives the training rows. It is cleared before every run
 * @param test — Destination database that receives the testing rows. It is cleared before every run
 * @impure has side effects / drives control flow
 */
declare function aiMlDatasetStratifiedSplit({ split?: float, labelColumn?: string, seed?: int, source: Struct, train: Struct, test: Struct }): void;


// === AI/ML/Metrics ===

/**
 * Calculate classification accuracy by comparing predictions to actual values
 * @param database — Database connection containing predictions and actuals
 * @param predictionsCol (optional) — Column name containing predicted values
 * @param actualsCol (optional) — Column name containing actual/true values
 * @returns result — Accuracy metrics including score and counts
 * @impure has side effects / drives control flow
 */
declare function mlEvalAccuracy({ database: Struct, predictionsCol?: string, actualsCol?: string }): Struct;

/**
 * Build confusion matrix and calculate precision, recall, and F1 score
 * @param database — Database connection containing predictions and actuals
 * @param predictionsCol (optional) — Column name containing predicted values
 * @param actualsCol (optional) — Column name containing actual/true values
 * @returns result — Confusion matrix with precision, recall, and F1 metrics
 * @impure has side effects / drives control flow
 */
declare function mlEvalConfusionMatrix({ database: Struct, predictionsCol?: string, actualsCol?: string }): Struct;

/**
 * Calculate MSE, RMSE, MAE, and R² for regression predictions
 * @param database — Database connection containing predictions and actuals
 * @param predictionsCol (optional) — Column name containing predicted float values
 * @param actualsCol (optional) — Column name containing actual/true float values
 * @returns result — Regression metrics (MSE, RMSE, MAE, R²)
 * @impure has side effects / drives control flow
 */
declare function mlEvalRegression({ database: Struct, predictionsCol?: string, actualsCol?: string }): Struct;

/**
 * Threshold-free evaluation of a binary classifier: area under the ROC curve, log loss and the curve points. This is the payoff for Logistic Regression producing calibrated probabilities instead of bare class labels.
 * @param database — Database connection containing the predicted probabilities and the true labels
 * @param probabilitiesCol (optional) — Column holding P(positive class) for each row, between 0 and 1 — the probability of the class named in Positive Label, NOT the probability of whichever class was predicted. No node writes this column for you: Predict in Database mode writes the predicted class only, and `confidence` is a field on the struct its Vector mode returns for one row, so build the column by looping rows through Vector mode. Convert as you go, because `confidence` is the winning class's probability: use it directly where the prediction is the positive class, and 1 - confidence elsewhere. A raw decision value or an uncalibrated score produces a meaningless curve.
 * @param actualsCol (optional) — Column holding the true binary label of each sample
 * @param positiveLabel (optional) — Value of the actuals column that counts as the positive class. Strings are compared literally, numbers numerically; booleans are always taken as-is.
 * @returns auc — Area under the ROC curve (0.5 = random, 1.0 = perfect)
 * @returns logLoss — Mean binary cross-entropy of the predicted probabilities (lower is better)
 * @returns result — AUC, log loss and the ROC curve points ordered by ascending false positive rate
 * @impure has side effects / drives control flow
 */
declare function mlRocAuc({ database: Struct, probabilitiesCol?: string, actualsCol?: string, positiveLabel?: string }): { auc: float, logLoss: float, result: Struct };

/**
 * Evaluate clustering quality: how much closer each sample sits to its own cluster than to the nearest other one (-1 to +1)
 * @param database — Database connection containing the feature vectors and their cluster assignments
 * @param featuresCol (optional) — Column holding the feature vectors the clustering was computed on. Distances are euclidean, so scale the features first if their ranges differ.
 * @param labelsCol (optional) — Column holding the cluster assignment of each sample, as a string name or a non-negative integer id
 * @param maxSamples (optional) — Upper bound on the samples used. The metric compares every sample with every other one, so the cost grows quadratically; larger sets are sub-sampled evenly.
 * @returns score — Mean silhouette score across all evaluated samples (-1 to +1, higher is better)
 * @returns nSamples — Number of samples the score was computed on after sub-sampling
 * @returns nClusters — Number of distinct clusters found in the cluster column
 * @impure has side effects / drives control flow
 */
declare function mlSilhouetteScore({ database: Struct, featuresCol?: string, labelsCol?: string, maxSamples?: int }): { score: float, nSamples: int, nClusters: int };


// === AI/ML/Model Info ===

/**
 * Extract per-feature importance from a Decision Tree, Random Forest or AdaBoost model
 * @param model — Trained tree model (Decision Tree, Random Forest or AdaBoost)
 * @param featureNames (optional) — Optional column labels in training order. Unnamed columns fall back to feature_<index>.
 * @returns result — Per-feature importance with leaf and depth statistics
 * @returns importances — Normalized importance per feature, in column order
 * @returns topFeature — Name of the most important feature
 * @impure has side effects / drives control flow
 */
declare function mlFeatureImportance({ model: Struct, featureNames?: string[] }): { result: Struct, importances: float[], topFeature: string };

/**
 * Extract cluster centroids from a trained KMeans model
 * @param model — Trained KMeans model
 * @returns result — Cluster centroids with metadata
 * @impure has side effects / drives control flow
 */
declare function mlGetKmeansCentroids({ model: Struct }): Struct;

/**
 * Extract coefficients and intercept from a trained Linear Regression model
 * @param model — Trained Linear Regression model
 * @returns result — Regression coefficients with intercept
 * @impure has side effects / drives control flow
 */
declare function mlGetLinearCoefficients({ model: Struct }): Struct;

/**
 * Get general information about any ML model
 * @param model — Any trained ML model
 * @returns info — Model information structure
 * @returns modelType — Model type as string
 * @impure has side effects / drives control flow
 */
declare function mlModelInfo({ model: Struct }): { info: Struct, modelType: string };


// === AI/ML/ONNX ===

/**
 * Extract a specific keypoint from a pose by index or name
 * @param pose — Pose detection to extract keypoint from
 * @param keypointIdx (optional) — Keypoint index (0-based)
 * @returns x — Keypoint X coordinate
 * @returns y — Keypoint Y coordinate
 * @returns confidence — Keypoint confidence score
 * @returns name — Keypoint name (if available)
 * @returns found — Whether the keypoint was found
 */
declare function extractKeypoint({ pose: Struct, keypointIdx?: int }): { x: float, y: float, confidence: float, name: string, found: bool };

/**
 * Extract feature vectors from images using ONNX models
 * @param model — ONNX Model Session
 * @param imageIn — Image Object
 * @param normalize (optional) — Normalize output to unit length
 * @returns features — Extracted feature vector
 * @returns dimensions — Feature vector dimensionality
 * @impure has side effects / drives control flow
 */
declare function featureExtraction({ model: Struct, imageIn: Struct, normalize?: bool }): { features: Struct, dimensions: int };

/**
 * Compare two feature vectors using cosine similarity or L2 distance
 * @param featuresA — First feature vector
 * @param featuresB — Second feature vector
 * @returns cosineSimilarity — Cosine similarity (-1 to 1, higher is more similar)
 * @returns l2Distance — Euclidean distance (lower is more similar)
 */
declare function featureSimilarity({ featuresA: Struct, featuresB: Struct }): { cosineSimilarity: float, l2Distance: float };

/**
 * Image Classification with ONNX-Models. Download models from: MobileNetV2 (https://github.com/onnx/models/tree/main/validated/vision/classification/mobilenet), SqueezeNet (https://github.com/onnx/models/tree/main/validated/vision/classification/squeezenet), ResNet (https://github.com/onnx/models/tree/main/validated/vision/classification/resnet), EfficientNet (https://github.com/onnx/models/tree/main/validated/vision/classification/efficientnet-lite4)
 * @param model — ONNX Model Session
 * @param imageIn — Image Object
 * @param mean (optional) — Image Mean for Normalization (per channel)
 * @param std (optional) — Image Standard Deviation for Normalization (per channel)
 * @param cropPct (optional) — Center Crop Percentage
 * @param softmax (optional) — Scale Outputs with Softmax
 * @returns predictions — Class Predictions
 * @impure has side effects / drives control flow
 */
declare function imageClassification({ model: Struct, imageIn: Struct, mean?: float[], std?: float[], cropPct?: float, softmax?: bool }): Struct[];

/**
 * Load ONNX Model from Path
 * @param path — Path ONNX File
 * @returns model — ONNX Model Session
 * @returns accelerated — Whether a GPU/NPU execution provider was configured; individual sessions may still fall back to CPU
 * @returns activeProvider — Execution providers configured in priority order, including CPU fallback
 * @impure has side effects / drives control flow
 */
declare function loadOnnx({ path: Struct }): { model: Struct, accelerated: bool, activeProvider: string };

/**
 * Object Detection in Images with ONNX-Models. Download models from: TinyYOLOv2 (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/tiny-yolov2), YOLO (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation), SSD-MobileNet (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/ssd-mobilenetv1)
 * @param model — ONNX Model Session
 * @param imageIn — Image Object
 * @param conf (optional) — Confidence Threshold
 * @param iou (optional) — Intersection Over Union Threshold for NMS
 * @param max (optional) — Maximum Number of Detections
 * @returns bboxes — Bounding Box Predictions
 * @impure has side effects / drives control flow
 */
declare function objectDetection({ model: Struct, imageIn: Struct, conf?: float, iou?: float, max?: int }): Struct[];

/**
 * Get ONNX model metadata (inputs, outputs, shapes)
 * @param path — Path to ONNX file
 * @returns metadata — Model metadata
 * @returns inputs — List of model inputs
 * @returns outputs — List of model outputs
 * @impure has side effects / drives control flow
 */
declare function onnxModelInfo({ path: Struct }): { metadata: Struct, inputs: Struct[], outputs: Struct[] };

/**
 * Get information about a loaded ONNX session
 * @param model — ONNX Model Session
 * @returns inputs — List of model inputs
 * @returns outputs — List of model outputs
 * @returns inputNames — Comma-separated input names
 * @returns outputNames — Comma-separated output names
 */
declare function onnxSessionInfo({ model: Struct }): { inputs: Struct[], outputs: Struct[], inputNames: string, outputNames: string };

/**
 * Detect human poses and keypoints using ONNX models. Download models from: YOLOv8-Pose (https://docs.ultralytics.com/models/yolov8/), MoveNet (https://tfhub.dev/google/movenet/), HRNet (https://github.com/OAID/TengineKit)
 * @param model — ONNX Model Session
 * @param imageIn — Image Object
 * @param conf (optional) — Minimum keypoint confidence threshold
 * @param maxPoses (optional) — Maximum number of poses to detect
 * @returns poses — Detected poses with keypoints
 * @impure has side effects / drives control flow
 */
declare function poseEstimation({ model: Struct, imageIn: Struct, conf?: float, maxPoses?: int }): Struct[];

/**
 * Segment images into semantic classes using ONNX models. Download models from: DeepLabV3 (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/duc), FCN (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/fcn)
 * @param model — ONNX Model Session
 * @param imageIn — Image Object
 * @param numClasses (optional) — Number of segmentation classes
 * @returns mask — Segmentation mask output
 * @impure has side effects / drives control flow
 */
declare function semanticSegmentation({ model: Struct, imageIn: Struct, numClasses?: int }): Struct;

/**
 * Release ONNX model from cache to free memory
 * @param model — ONNX Model Session to unload
 * @returns success — Whether the model was successfully unloaded
 * @impure has side effects / drives control flow
 */
declare function unloadOnnx({ model: Struct }): bool;


// === AI/ML/ONNX/Audio ===

/**
 * Convert audio to mel spectrogram for speech models
 * @param audio — Input audio (16kHz mono)
 * @param nMels (optional) — Number of mel bands
 * @param hopLength (optional) — Hop length in samples
 * @param nFft (optional) — FFT window size
 * @returns spectrogram — Mel spectrogram [n_mels, time]
 * @returns frames — Number of time frames
 * @impure has side effects / drives control flow
 */
declare function audioToMelSpectrogram({ audio: Struct, nMels?: int, hopLength?: int, nFft?: int }): { spectrogram: any, frames: int };

/**
 * Load audio file for processing
 * @param path — Path to audio file
 * @returns audio — Loaded audio data
 * @returns sampleRate — Audio sample rate
 * @returns duration — Duration in seconds
 * @impure has side effects / drives control flow
 */
declare function loadAudio({ path: Struct }): { audio: Struct, sampleRate: int, duration: float };

/**
 * Detect speech segments in audio. Download Silero VAD model from: https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx
 * @param model — ONNX VAD Model
 * @param audio — Input audio data
 * @param threshold (optional) — Speech probability threshold
 * @param minSpeechMs (optional) — Minimum speech duration (ms)
 * @param minSilenceMs (optional) — Minimum silence duration (ms)
 * @returns result — VAD result
 * @returns segments — Speech segments
 * @impure has side effects / drives control flow
 */
declare function onnxVad({ model: Struct, audio: Struct, threshold?: float, minSpeechMs?: int, minSilenceMs?: int }): { result: Struct, segments: any };

/**
 * Resample audio to target sample rate
 * @param audio — Input audio
 * @param targetRate (optional) — Target sample rate
 * @param toMono (optional) — Convert to mono
 * @returns audioOut — Resampled audio
 * @impure has side effects / drives control flow
 */
declare function resampleAudio({ audio: Struct, targetRate?: int, toMono?: bool }): Struct;

/**
 * Trim audio to speech segments from VAD
 * @param audio — Input audio
 * @param segments — Speech segments from VAD
 * @param padding (optional) — Padding around segments (seconds)
 * @returns clips — Trimmed audio clips
 * @impure has side effects / drives control flow
 */
declare function trimAudio({ audio: Struct, segments: any, padding?: float }): any;


// === AI/ML/ONNX/Batch ===

/**
 * Run ONNX inference on multiple images in batches
 * @param model — ONNX Model Session
 * @param images — List of images to process
 * @param batchSize (optional) — Number of images per batch
 * @param inputSize (optional) — Model input size
 * @param normalize (optional) — Apply ImageNet normalization
 * @returns results — Raw output tensors per image
 * @returns batchResult — Batch processing summary
 * @impure has side effects / drives control flow
 */
declare function onnxBatchImageInference({ model: Struct, images: any, batchSize?: int, inputSize?: int, normalize?: bool }): { results: any, batchResult: Struct };


// === AI/ML/ONNX/Face ===

/**
 * Compare two face embeddings for similarity
 * @param embeddingA — First face embedding
 * @param embeddingB — Second face embedding
 * @param threshold (optional) — Match threshold (cosine similarity)
 * @returns isMatch — Whether faces match
 * @returns similarity — Cosine similarity score
 * @returns distance — Euclidean distance
 * @impure has side effects / drives control flow
 */
declare function compareFaces({ embeddingA: Struct, embeddingB: Struct, threshold?: float }): { isMatch: bool, similarity: float, distance: float };

/**
 * Crop detected faces from image
 * @param image — Source image
 * @param faces — Detected faces
 * @param margin (optional) — Margin around face (fraction)
 * @returns crops — Cropped face images
 * @impure has side effects / drives control flow
 */
declare function cropFaces({ image: Struct, faces: any, margin?: float }): any;

/**
 * Detect faces and extract embeddings, gender and age using a face_id analyzer
 * @param analyzer — Face analyzer handle
 * @param image — Input Image
 * @param maxFaces (optional) — Maximum number of faces to embed and analyze
 * @returns faces — Analyzed faces
 * @returns count — Number of detected faces
 * @impure has side effects / drives control flow
 */
declare function faceIdAnalyze({ analyzer: Struct, image: Struct, maxFaces?: int }): { faces: Struct[], count: int };

/**
 * Load a face_id analyzer (SCRFD detector + ArcFace embedder + gender/age). Weights are verified and cached when a session identity is first built; equivalent analyzers reuse process-wide sessions.
 * @param cacheDir — FlowPath used when this analyzer identity needs to build its ONNX sessions. If it is already resident, an alternate cache directory is not populated.
 * @param detectorUrl (optional) — Immutable SCRFD detector weights URL
 * @param detectorSha256 (optional) — Required SHA-256 checksum for the detector weights
 * @param embedderUrl (optional) — Immutable ArcFace recognition weights URL
 * @param embedderSha256 (optional) — Required SHA-256 checksum for the recognition weights
 * @param genderAgeUrl (optional) — Immutable gender & age estimation weights URL
 * @param genderAgeSha256 (optional) — Required SHA-256 checksum for the gender & age weights
 * @param inputSize (optional) — Square detector input size
 * @param scoreThreshold (optional) — Detector confidence threshold
 * @param iouThreshold (optional) — Detector non-maximum-suppression IoU threshold
 * @returns analyzer — Cached face analyzer handle
 * @impure has side effects / drives control flow
 */
declare function faceIdLoadAnalyzer({ cacheDir: Struct, detectorUrl?: string, detectorSha256?: string, embedderUrl?: string, embedderSha256?: string, genderAgeUrl?: string, genderAgeSha256?: string, inputSize?: int, scoreThreshold?: float, iouThreshold?: float }): Struct;

/**
 * Release a cached face analyzer and its three ONNX sessions. Equivalent analyzer handles share the same cache entry and are invalidated together.
 * @param analyzer — Face analyzer handle to unload
 * @returns success — Whether a face analyzer cache entry was removed
 * @impure has side effects / drives control flow
 */
declare function faceIdUnloadAnalyzer({ analyzer: Struct }): bool;

/**
 * Detect faces in images. Download models from: UltraFace (https://github.com/onnx/models/tree/main/validated/vision/body_analysis/ultraface), RetinaFace (https://huggingface.co/arnabdhar/retinaface-onnx), SCRFD (https://huggingface.co/onnx-community/scrfd_10g_bnkps)
 * @param model — ONNX Face Detection Model
 * @param image — Input Image
 * @param threshold (optional) — Detection confidence threshold
 * @param nmsThreshold (optional) — Non-maximum suppression threshold
 * @param inputSize (optional) — Model input size
 * @returns faces — Detected faces
 * @returns count — Number of detected faces
 * @impure has side effects / drives control flow
 */
declare function onnxFaceDetection({ model: Struct, image: Struct, threshold?: float, nmsThreshold?: float, inputSize?: int }): { faces: any, count: int };

/**
 * Extract face embedding for recognition. Download models from: ArcFace (https://huggingface.co/onnx-community/arcface_torch/tree/main), FaceNet (https://huggingface.co/rocca/facenet-onnx)
 * @param model — ONNX Face Embedding Model
 * @param image — Aligned face image
 * @param inputSize (optional) — Model input size (typically 112 or 160)
 * @returns embedding — Face embedding vector
 * @impure has side effects / drives control flow
 */
declare function onnxFaceEmbedding({ model: Struct, image: Struct, inputSize?: int }): Struct;


// === AI/ML/ONNX/NLP ===

/**
 * Extract entities for any labels you name at runtime, with no fixed label set and no retraining. Load a GLiNER ONNX export (e.g. https://huggingface.co/onnx-community/gliner_small-v2.1, gliner_multi-v2.1, gliner_medium_news-v2.1, gliner_multi_pii-v1, NuNER_Zero) plus the tokenizer.json from the same repository. For models with a fixed label set, use the Named Entity Recognition node instead.
 * @param model — ONNX GLiNER Model Session
 * @param tokenizer — HuggingFace tokenizer.json from the same model repository
 * @param text — Input text to analyze for named entities
 * @param labels — Entity types to look for, in plain language (e.g. person, company, medication, invoice number)
 * @param threshold (optional) — Minimum confidence for a span to be reported (0.0-1.0)
 * @param maxWidth (optional) — Longest entity in words. Must match the model's max_width from gliner_config.json (12 for most GLiNER models, 1 for NuNER Zero)
 * @param multiLabel (optional) — Report every label that clears the threshold for a span instead of only the best one
 * @param mergeAdjacent (optional) — Join neighbouring same-label entities separated only by whitespace. Required for token-level models such as NuNER Zero, which score one word at a time
 * @returns result — Full zero-shot result with entities and the labels that were requested
 * @returns entities — Extracted entities as array
 * @returns entityCount — Number of entities found
 * @impure has side effects / drives control flow
 */
declare function onnxGliner({ model: Struct, tokenizer: Struct, text: string, labels: string[], threshold?: float, maxWidth?: int, multiLabel?: bool, mergeAdjacent?: bool }): { result: Struct, entities: Struct[], entityCount: int };

/**
 * Extract named entities (persons, organizations, locations, dates, etc.) from text using ONNX models. Supports BERT, RoBERTa, and other transformer-based NER models with automatic tokenization. Download models from: BERT-base-NER (https://huggingface.co/dslim/bert-base-NER), Multilingual NER (https://huggingface.co/Davlan/bert-base-multilingual-cased-ner-hrl), spaCy NER (https://huggingface.co/spacy). Text longer than the model's window is split into overlapping chunks rather than truncated, so entities are found throughout a long document. Download tokenizer.json and config.json from the same model repository — config.json carries the id2label mapping that names the entity types and the sequence length the model accepts.
 * @param model — ONNX NER Model Session
 * @param tokenizer — HuggingFace tokenizer.json file for BERT/RoBERTa tokenization. Download from the same model repository.
 * @param config — HuggingFace config.json of the model. Supplies the id2label mapping that decides which class index means which entity type, and max_position_embeddings, which sets how many tokens fit in one window. Left empty, the node looks for config.json next to the tokenizer. Strongly recommended: label orderings differ between models of the same size, and a wrong one mislabels every entity.
 * @param text — Input text to analyze for named entities
 * @param labels — Entity label names in model output order (e.g. ['O', 'B-PER', 'I-PER', 'B-ORG', ...]). Overrides the Config pin. If both are empty, the node falls back to the CoNLL-2003 ordering of dslim/bert-base-NER.
 * @param scheme (optional) — Tagging scheme: BIO, BIOES, IOB, or BILOU
 * @param threshold (optional) — Minimum confidence threshold for entity extraction (0.0-1.0)
 * @returns result — Full NER result with entities and token predictions
 * @returns entities — Extracted named entities as array
 * @returns entityCount — Number of entities found
 * @impure has side effects / drives control flow
 */
declare function onnxNer({ model: Struct, tokenizer: Struct, config: Struct, text: string, labels: string[], scheme?: Struct, threshold?: float }): { result: Struct, entities: Struct[], entityCount: int };


// === AI/ML/ONNX/OCR ===

/**
 * Crop detected text regions from image for recognition
 * @param image — Source image
 * @param regions — Detected text regions
 * @param padding (optional) — Padding around regions (pixels)
 * @returns crops — Cropped region images
 * @impure has side effects / drives control flow
 */
declare function cropTextRegions({ image: Struct, regions: any, padding?: int }): any;

/**
 * Detect text regions in images. Download models from: CRAFT (https://huggingface.co/quocanh34/craft_text_detection_onnx), DBNet (https://huggingface.co/Xenova/dbnet_resnet50_onnx), EAST (https://www.dropbox.com/s/r2ingd0l3zt8hxs/frozen_east_text_detection.tar.gz)
 * @param model — ONNX Text Detection Model
 * @param image — Input Image
 * @param threshold (optional) — Detection confidence threshold
 * @param inputSize (optional) — Model input size
 * @returns regions — Detected text regions
 * @returns count — Number of detected regions
 * @impure has side effects / drives control flow
 */
declare function onnxTextDetection({ model: Struct, image: Struct, threshold?: float, inputSize?: int }): { regions: any, count: int };

/**
 * Recognize text from cropped text regions. Download models from: CRNN (https://huggingface.co/Xenova/crnn_onnx), TrOCR (https://huggingface.co/microsoft/trocr-base-printed), PaddleOCR (https://huggingface.co/aapot/paddleocr-onnx)
 * @param model — ONNX Text Recognition Model
 * @param image — Cropped text region image
 * @param charset (optional) — Character set for decoding
 * @param inputHeight (optional) — Model expected input height
 * @returns result — Recognition result
 * @returns text — Recognized text string
 * @impure has side effects / drives control flow
 */
declare function onnxTextRecognition({ model: Struct, image: Struct, charset?: string, inputHeight?: int }): { result: Struct, text: string };


// === AI/ML/ONNX/Vision ===

/**
 * Convert depth map to rainbow-colored visualization
 * @param depthMap — Input depth map
 * @returns coloredImage — Rainbow-colored depth visualization
 * @impure has side effects / drives control flow
 */
declare function depthColorize({ depthMap: Struct }): Struct;

/**
 * Convert depth map to 3D point cloud coordinates
 * @param depthMap — Input depth map
 * @param focalLength (optional) — Camera focal length (pixels)
 * @param scale (optional) — Depth scale factor
 * @returns points — 3D point coordinates [x, y, z]
 * @returns pointCount — Number of points
 * @impure has side effects / drives control flow
 */
declare function depthToPointCloud({ depthMap: Struct, focalLength?: float, scale?: float }): { points: any, pointCount: int };

/**
 * Estimate depth from a single image using ONNX models. Download models from: MiDaS (https://github.com/isl-org/MiDaS/releases), DPT (https://huggingface.co/Intel/dpt-large/tree/main), Depth Anything (https://huggingface.co/depth-anything/Depth-Anything-V2-Small/tree/main)
 * @param model — ONNX Depth Model Session
 * @param image — Input Image
 * @param provider (optional) — Model provider type
 * @param inputSize (optional) — Model input size (default 384 for MiDaS)
 * @returns depthMap — Estimated depth map
 * @returns depthImage — Grayscale depth visualization
 * @impure has side effects / drives control flow
 */
declare function onnxDepthEstimation({ model: Struct, image: Struct, provider?: Struct, inputSize?: int }): { depthMap: Struct, depthImage: Struct };


// === AI/ML/Ordinal ===

/**
 * Fit/Train an ordinal model that compares each level with the one directly below it: `log( P(level k+1) / P(level k) ) = contrast_k + x . beta`. Its coefficients answer `what does one more unit of this feature do to my rating?` - `exp(coefficient)` is the factor on the odds of scoring one level higher rather than staying put, the same factor at every step. That is NOT what Train Ordinal Model (Proportional Odds) reports: a cumulative coefficient is the log odds ratio of everything AT OR BELOW a cut point against everything above it, pooling levels instead of comparing two neighbours. The same fitted number therefore means different things in the two families, and since one shared coefficient applies once per step here, the bottom-to-top effect is (levels - 1) times the per-step effect. Pick this for ratings, severity grades and Likert answers, where the question really is about one step; pick proportional odds when the question is about crossing a threshold (`does this case escalate past level 2?`). Fitted by penalized maximum likelihood over all levels jointly, so per-level probabilities are calibrated and the Predict node returns a confidence. Scale your features first with the Fit Feature Scaler node: this is a gradient fit, and unscaled columns make it converge slowly or not at all.
 * @param source (optional) — Choose which backend supplies the training data
 * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Levels listed here but never seen in training still keep their slot in the ordering, so the contrasts stay comparable across runs.
 * @param alpha (optional) — Strength of the L2 penalty on the shared coefficients. The level contrasts are never penalized: shrinking those would pull neighbouring levels toward equal frequency, which asserts something about your data rather than limiting model complexity. 0 fits unpenalized. Raise it when the fit diverges or the coefficients blow up.
 * @param maxIterations (optional) — Iteration cap for the Adam optimizer. Training stops here even if the objective is still moving, which is reported on the Converged pin.
 * @param tolerance (optional) — Relative change in the objective below which training stops. Smaller values fit tighter but need more iterations; 0 always runs the full iteration budget.
 * @param learningRate (optional) — Adam step size. Lower it if training oscillates or produces non-finite values; raise it if the model has not converged within Max Iterations. Level scores here carry a factor of the level index, so a badly scaled step travels further than it would in a cumulative fit.
 * @returns model — Thread-safe handle to the trained adjacent-category model. Predictions come back as your original level labels, and because the fit maximizes a likelihood the Predict node also returns a per-level confidence.
 * @returns levels — The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.
 * @returns converged — False when the optimizer hit Max Iterations before the objective settled. The model is still usable but under-fitted.
 * @returns coefficients — The shared per-feature coefficients together with the level contrasts, both of them PER-STEP quantities: `exp(coefficient)` multiplies the odds of landing one level higher rather than on the current one, which is a single step and not the cumulative `above this cut` odds ratio a proportional-odds model prints. The struct also carries `bottom_to_top_effect`, the same coefficient times (levels - 1), which is the magnitude to quote when someone asks about the full range. The contrasts are the same log odds at a zero score, one per adjacent pair; unlike cumulative cut points they are free intercepts and may DECREASE.
 * @impure has side effects / drives control flow
 */
declare function fitOrdinalAdjacentCategory({ source?: string, classOrder?: string, alpha?: float, maxIterations?: int, tolerance?: float, learningRate?: float }): { model: Struct, levels: Struct, converged: bool, coefficients: Struct };

/**
 * Fit/Train a continuation-ratio model on an ORDERED target that is really a process that can halt. It fits K-1 sub-models, where sub-model k answers `given this row reached level k, did it STOP there?`, so the model describes a progression through the levels instead of placing cut points on a latent scale. Reach for it when the levels are genuinely sequential and each one had to be passed to get to the next: escalation tiers, disease stages, how far a signup funnel got, how far an incident escalated before it was contained. Each sub-model carries its own coefficient vector, so nothing assumes proportional odds, and the per-level probabilities are exact by the chain rule rather than differences of two fits. The cost is strictness: because each sub-model is conditioned on having reached its level, EVERY level must occur in the training data, middle ones included. Scale your features first with the Fit Feature Scaler node: these are gradient fits, and unscaled columns make them converge slowly or not at all.
 * @param source (optional) — Choose which backend supplies the training data
 * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Unlike the other ordinal nodes, a level you list here that never occurs in the data is rejected instead of merely left unpredicted: its sub-model would have no rows to separate.
 * @param link (optional) — The CDF each conditional stopping probability is read through. CLogLog is the standout pairing here: with it this model IS the discrete-time proportional-hazards (grouped survival) model, each sub-model's output is the hazard of stopping at that step, and a shared feature effect multiplies every hazard by the same factor — so for `how long / how far until something stopped` targets, pick CLogLog and read the fit as a survival model. Logit gives conditional log-odds, the classical continuation-ratio logit, and is the safe default. Probit assumes a normal latent variable per step. Cauchit is heavy-tailed, so extreme rows pull each sub-model far less.
 * @param alpha (optional) — Strength of the L2 penalty on each sub-model's coefficients; the intercepts are never penalized. Because the penalty is a fixed amount added to a summed log-likelihood, one value shrinks the high levels harder than the low ones — which is what you want, since those are the sub-models fitted on the fewest rows. Raise it when Subset Sizes shows a thin top end.
 * @param maxIterations (optional) — Iteration cap for the Adam optimizer, applied to EACH sub-model separately. A single sub-model stopping here makes Converged false.
 * @param tolerance (optional) — Relative change in a sub-model's objective below which its fit stops. The test is relative, so it means the same thing on the large bottom subset and the small top one. 0 always runs the full iteration budget.
 * @param learningRate (optional) — Adam step size, shared by every sub-model. Lower it if training oscillates or produces non-finite values; raise it if the model has not converged within Max Iterations.
 * @returns model — Thread-safe handle to the trained continuation-ratio model. Predictions come back as your original level labels, and the per-level probabilities behind them sum to exactly 1 because the chain rule telescopes.
 * @returns levels — The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.
 * @returns subsetSizes — How many training rows each sub-model actually saw, lowest level first: entry k counts the rows that reached level k. It only ever decreases, so the LAST entry is the evidence behind your top level — the honest measure of how much to trust the high end of the fit. A small tail there means the top coefficients are noise, not a subtle effect.
 * @returns converged — True only when EVERY sub-model's objective settled before Max Iterations. One stubborn sub-model — usually the top one, fitted on the fewest rows — makes it false; the run log names which levels stalled.
 * @impure has side effects / drives control flow
 */
declare function fitOrdinalContinuationRatio({ source?: string, classOrder?: string, link?: string, alpha?: float, maxIterations?: int, tolerance?: float, learningRate?: float }): { model: Struct, levels: Struct, subsetSizes: int[], converged: bool };

/**
 * Fit/Train an ordinal model by decomposition: the ordered target is cut K-1 times (`is the level above this cut?`) and each cut is handed to an ordinary binary classifier, with the predicted level read back as the number of cuts answered yes. This is the one ordinal trainer here that is not linear in the features, so reach for it when the boundary between levels bends in a way the Proportional Odds and Ridge trainers cannot follow. The price is that the K-1 sub-models are fitted independently: there is no single latent scale, no coefficient vector to read a direction off, and no calibrated per-level probabilities - use Proportional Odds when you need those. Every declared level must occur in the training data at the bottom and at the top of the ordering, otherwise a cut has only one class and cannot be fitted. A Random Forest base is the sturdiest choice and by far the costliest: each cut grows its own full forest, so training costs K-1 forests and the saved model carries every tree of every one of them.
 * @param source (optional) — Choose which backend supplies the training data
 * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Listing a level that never occurs at either end of the ordering makes its cut unfittable and is rejected.
 * @param baseLearner (optional) — Which binary classifier is fitted for each of the K-1 cuts. Decision Tree follows non-linear, non-monotone boundaries and needs no feature scaling, at the cost of overfitting when left deep. Gaussian Naive Bayes is far cheaper and stays stable when rows are few relative to columns, but assumes the features are independent and roughly normal on each side of a cut. Random Forest bags many trees per cut and averages away most of a single tree's variance, usually making it the strongest option here - but it fits one entire forest per cut, so both the training time and the size of the saved model are multiplied by K-1.
 * @returns model — Thread-safe handle to the trained decomposition. Predictions come back as your original level labels.
 * @returns levels — The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.
 * @impure has side effects / drives control flow
 */
declare function fitOrdinalFrankHall({ source?: string, classOrder?: string, baseLearner?: string }): { model: Struct, levels: Struct };

/**
 * Fit/Train a proportional-odds model on a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). Use this instead of a classifier, which treats the levels as unrelated names and so counts predicting `low` for `high` as no worse than predicting `medium`. Use it instead of a regressor, which treats the levels as real numbers and so invents distances the levels do not carry (`high` is not exactly twice `medium`). The model learns one coefficient vector plus ordered cut points, which keeps predictions monotone in the score and, under the default loss, yields calibrated per-level probabilities. Link Function, Loss and Margin widen it to the whole threshold-model family, up to support vector ordinal regression, while Free Features relaxes the shared coefficient into one slope per cut point. Scale your features first with the Fit Feature Scaler node: this is a gradient fit, and unscaled columns make it converge slowly or not at all.
 * @param source (optional) — Choose which backend supplies the training data
 * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one.
 * @param link (optional) — The CDF sitting behind the cut points, i.e. which latent distribution you assume produced the levels. Logit gives the proportional-odds model and coefficients that read as log odds ratios. Probit assumes a normally distributed latent variable and is the convention in econometrics and the social sciences. CLogLog is asymmetric — it leaves the bottom level quickly and approaches the top one slowly — which is the right shape for `time until something escalates` targets. Cauchit is heavy-tailed, so extreme rows pull the fit far less than they do under Logit or Probit. Applies to the CumulativeLink loss only: the two threshold losses use a logistic margin and ignore this.
 * @param loss (optional) — What the optimizer actually minimizes. CumulativeLink maximizes the likelihood of each level and is the ONLY choice that carries a probability model — the confidence value on the Predict node comes from it. AllThreshold penalizes every cut point that falls on the wrong side of the observation, ImmediateThreshold only the two bracketing it; both drop the proportional-odds assumption and are often more robust when it fails, but they fit cut-point placement rather than a likelihood, so the resulting model yields NO per-level probabilities and Predict returns no confidence.
 * @param margin (optional) — Shape of the penalty a cut point pays for sitting on the wrong side of an observation. Hinge charges nothing once the cut point clears the margin, so only the observations NEAR a cut point influence the fit at all: Hinge together with the AllThreshold loss IS support vector ordinal regression (Chu & Keerthi's implicit-constraint SVOR), and with ImmediateThreshold it is the explicit-constraint variant. SquaredHinge is the differentiable version of that kink — smoother gradients, but distant violations are punished quadratically, so single outliers drag the cut points. Logistic is smooth everywhere and charges even well-placed cut points a little. IGNORED by the default CumulativeLink loss, which maximizes a likelihood and has no margin.
 * @param freeFeatures (optional) — Comma-separated feature INDICES (0-based, e.g. `0, 3`) that get their own coefficient at EVERY cut point instead of one shared across all of them — the partial proportional-odds model. Empty is the standard model, where a single slope describes every cut point; that is an assumption. Free a feature when you suspect it violates it, then check the Effective Coefficients output: a feature whose per-cut slopes barely differ gained nothing by being freed. Freeing only the ones that do differ keeps every other feature parsimonious. Listing every index gives the fully generalized ordinal model. The price shows up on Crossing Rate: unconstrained per-cut slopes let the cumulative curves cross, which is no longer a valid probability model.
 * @param alpha (optional) — Strength of the L2 penalty on the coefficients; the cut points are never penalized. 0 fits unpenalized. Raise it when the fit diverges or the coefficients blow up.
 * @param maxIterations (optional) — Iteration cap for the Adam optimizer. Training stops here even if the objective is still moving, which is reported on the Converged pin.
 * @param tolerance (optional) — Relative change in the objective below which training stops. Smaller values fit tighter but need more iterations; 0 always runs the full iteration budget.
 * @param learningRate (optional) — Adam step size. Lower it if training oscillates or produces non-finite values; raise it if the model has not converged within Max Iterations.
 * @returns model — Thread-safe handle to the trained proportional-odds model. Predictions come back as your original level labels.
 * @returns levels — The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.
 * @returns converged — False when the optimizer hit Max Iterations before the objective settled. The model is still usable but under-fitted.
 * @returns crossingRate — Share of training rows (0.0 to 1.0) whose cumulative curves crossed, i.e. where the fit put P(y <= k) ABOVE P(y <= k+1) and so implied a negative probability for a level. Always 0.0 without Free Features, because a shared slope cannot cross. Anything above 0 means the generalized fit is no longer a clean probability model: prediction clamps and renormalizes so nothing downstream sees a negative number, but the per-level probabilities stop being trustworthy — free fewer features, or go back to the shared model.
 * @returns effectiveCoefficients — The coefficient of every feature at every cut point, one row per cut point from lowest to highest, next to the cut points themselves. Shared features repeat the same value down every row; freed ones vary, and the reported spread (largest minus smallest over the cut points) is how you tell whether freeing a feature bought anything — a spread near zero means one shared slope fitted it just as well and the extra parameters were wasted.
 * @impure has side effects / drives control flow
 */
declare function fitOrdinalLogistic({ source?: string, classOrder?: string, link?: string, loss?: string, margin?: string, freeFeatures?: string, alpha?: float, maxIterations?: int, tolerance?: float, learningRate?: float }): { model: Struct, levels: Struct, converged: bool, crossingRate: float, effectiveCoefficients: Struct };

/**
 * Fit/Train a NEURAL ordinal model on a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). This is the only trainer in the catalog that is BOTH non-linear in the features AND yields calibrated, rank-consistent per-level probabilities: Frank & Hall is non-linear but votes with K-1 independent classifiers and therefore carries no probability model, while every other ordinal node here is linear in the features. A small network feeds one of two rank-consistent heads, CORAL or CORN, and both are built so that P(y > k) can never rise with k for ANY parameter values — so the level probabilities are non-negative and sum to 1 with nothing patched up afterwards. THE HONEST LIMIT: leave Hidden Layers EMPTY and CORAL becomes exactly Train Ordinal Model (Proportional Odds) with Loss = AllThreshold and Margin = Logistic, and CORN becomes exactly Train Ordinal Model (Continuation Ratio) — the same objective in the same parameters. The hidden layers are the entire contribution, so if your problem is linear in the features prefer those nodes: convex objective, no seed dependence, readable coefficients, better tested. Reach for this one when the level is genuinely not monotone in the features (a boundary that bends back on itself, which no linear ordinal model can represent at all). Two costs come with the network: it has far more parameters than a linear model and so needs far more rows — check the Architecture output — and the objective is not convex, so the Seed changes the fit. Scale your features first with the Fit Feature Scaler node; unscaled columns make this converge slowly or not at all.
 * @param source (optional) — Choose which backend supplies the training data
 * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Note that a declared level the training data never reaches is fine for CORAL but rejected by CORN, whose task for that level would have no rows to fit.
 * @param head (optional) — Which rank-consistent head sits on the network. Coral shares ONE latent score across every cut point and lets the cut points differ only by an ordered bias, so a row's whole position on the scale is a single number: fewer parameters, lower variance, and the right choice when the levels really are separated by one underlying quantity or when the top levels are thin. Corn instead asks each step conditionally — given the row reached this level, does it go further? — and gives every step its own weights on the shared representation, which suits a target that is a genuine sequential process (escalation tiers, disease stages, how far a funnel got). Its price is data: step k trains only on the rows that reached level k, so the higher steps rest on the fewest rows, and Corn refuses outright to fit a declared level that nothing reaches.
 * @param hiddenLayers (optional) — Comma-separated hidden layer widths from the input side, e.g. `16, 8` for two layers. This is the ONLY thing this node adds over the linear ordinal trainers: an EMPTY value collapses the model to its linear equivalent exactly — Coral becomes the All-Threshold proportional-odds fit, Corn becomes the continuation-ratio fit — so if you want an empty value you want one of those simpler, better-tested nodes instead. Wider and deeper buys a boundary that can bend, and costs parameters that have to be paid for in rows: compare the Architecture output's parameter count against your row count. Every width must be at least 1; a zero-width layer would disconnect the head from the features and fit a constant.
 * @param activation (optional) — Non-linearity between the hidden layers. The head itself is always linear, and this has no effect at all when Hidden Layers is empty. Relu is cheap, and its piecewise-linear folds are exactly what let a small network represent a level that is not monotone in the features. Tanh is smooth and bounded, which often trains more gently on small, well-scaled data, but it saturates on large inputs and then passes almost no gradient — one more reason to scale the features first.
 * @param alpha (optional) — Strength of the L2 penalty on the WEIGHT matrices. Biases and the head's ordering parameters are never penalized: shrinking those would drag the level cut points together and quietly collapse adjacent levels, which changes the model rather than its variance. Raise it when the network memorizes the training rows or the loss blows up; 0 fits unpenalized.
 * @param maxIterations (optional) — Iteration cap for the Adam optimizer; each iteration is one full pass over the training set. Training stops here even if the loss is still falling, which is reported on the Converged pin. A network usually needs noticeably more iterations than the linear ordinal fits.
 * @param tolerance (optional) — Relative change in the loss below which training stops. Smaller values fit tighter and cost iterations; 0 always spends the whole iteration budget.
 * @param learningRate (optional) — Adam step size. Lower it if the loss oscillates or goes non-finite, raise it if the model has not converged within Max Iterations. A network wants a smaller step than the linear ordinal fits, because a hidden layer compounds every step.
 * @param seed (optional) — Seed for the weight initialization, which is the only randomness in the fit. The objective is NOT convex, so the seed genuinely changes the model you get and an unlucky one can leave the fit in a poor local optimum: refit with two or three seeds to see whether the result is stable. The same seed, data and hyperparameters reproduce a fit exactly.
 * @returns model — Thread-safe handle to the trained neural ordinal model. Predictions come back as your original level labels, and unlike the threshold losses of the proportional-odds node this family always carries per-level probabilities, so the Predict node reports a confidence.
 * @returns levels — The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.
 * @returns converged — False when the optimizer hit Max Iterations before the loss settled. The model is still usable but under-fitted, which on a network is more common than on the linear ordinal fits.
 * @returns architecture — What was actually built: the head, the activation, the hidden layer widths as fitted, and the total parameter count next to the number of training rows. Read the rows-per-parameter figure before you trust a training score — with fewer rows than parameters the network can reproduce the training labels outright. Empty hidden layers here means the fit was the linear equivalent, and a simpler ordinal node would have done the same job.
 * @impure has side effects / drives control flow
 */
declare function fitOrdinalNeural({ source?: string, classOrder?: string, head?: string, hiddenLayers?: string, activation?: string, alpha?: float, maxIterations?: int, tolerance?: float, learningRate?: float, seed?: int }): { model: Struct, levels: Struct, converged: bool, architecture: Struct };

/**
 * Fit/Train an ordinal model the cheap way: ridge-regress the level rank on the features, then cut the score at thresholds learned from the training distribution instead of rounding it. Closed-form, so it stays fast exactly where the proportional-odds model gets expensive - many levels, many features, or when you just want a quick ordinal baseline to beat. It also degrades gracefully when the proportional-odds assumption does not hold. Unlike the proportional-odds model it yields no probabilities: you get the predicted level and the latent score behind it, nothing calibrated.
 * @param source (optional) — Choose which backend supplies the training data
 * @param classOrder (optional) — Level labels from LOWEST to HIGHEST, comma separated - e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is already the one you want (`1, 2, 10` sorts as numbers, not as text). Non-numeric labels carry no inferable order, so training fails rather than guessing unless you list them here.
 * @param alpha (optional) — Strength of the L2 penalty. Must be strictly greater than 0: the penalty is added to the diagonal of the normal equations and is the only thing keeping them positive definite, so the Cholesky solve has a unique answer even with collinear or wide features. Larger values shrink the coefficients harder.
 * @returns model — Thread-safe handle to the trained ordinal ridge model. Predictions come back as the original level labels.
 * @returns levels — The resolved level order the model was trained on, lowest first, plus whether that order came from `Class Order` or from reading the labels as numbers. Check it before trusting the model - a wrong order trains a wrong model without ever failing.
 * @returns coefficients — Fitted coefficients and intercept on the rank scale. The SIGN tells you which way a feature pushes the level: positive moves samples toward the higher levels, negative toward the lower ones. The magnitude is only comparable across features when they share a scale.
 * @impure has side effects / drives control flow
 */
declare function fitOrdinalRidge({ source?: string, classOrder?: string, alpha?: float }): { model: Struct, levels: Struct, coefficients: Struct };

/**
 * Evaluate predictions for an ordered target with distance-aware metrics. Plain accuracy is inadequate here: it treats "predicted high when the truth was medium" exactly as harshly as "predicted low", so a model that is reliably one level off scores like one that guesses. Quadratic weighted kappa is the standard headline metric because it weights every miss by how far off it was and corrects for chance agreement, but it answers only one of three questions: the linear kappa and the macro-averaged error say how far off the model is under a different cost structure and on the rare levels, while Kendall's tau-b and the Spearman correlation say whether it orders the rows correctly at all.
 * @param database — Database connection containing the predicted levels and the true levels
 * @param predictionsCol (optional) — Column holding the predicted level of each row. The labels must be the same ones the actuals column uses, since both columns are ranked against one shared level order.
 * @param actualsCol (optional) — Column holding the true level of each row. When no Class Order is given, the level order is inferred from this column, and a predicted level that never occurs here is an error rather than a silent extra rank.
 * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order (sorting them alphabetically would rank high < low < medium), so they have to be listed here.
 * @returns quadraticWeightedKappa — Headline ordinal metric: chance-corrected agreement weighted by the squared level distance. 1.0 perfect, 0.0 chance, negative worse than chance.
 * @returns linearWeightedKappa — The same chance-corrected agreement with every level of distance costing the same. Read this one instead of the quadratic kappa when a level is a level — grading scales, severity tiers, anything where two steps off is exactly twice as bad as one. Quadratic weighting charges a near miss only a quarter of a two-level miss, so it flatters a model that merely hovers next to the truth; where that discount is not real, this is the honest number and it will be the lower of the two.
 * @returns meanAbsoluteRankError — Average miss in levels. 0.0 is perfect, 1.0 means being off by one level on average.
 * @returns macroMeanAbsoluteError — The mean absolute rank error computed per true level and averaged with one vote per level. Look here whenever the levels are imbalanced: the plain error averages over rows, so the majority level speaks for the model and a predictor that collapses onto it still scores well while missing every rare level. This metric gives the rare levels equal weight, so it is the one that moves when that happens. Levels absent from the actuals are skipped rather than counted as perfect.
 * @returns accuracyExact — Share of predictions hitting the exact level. Reported for reference; it ignores how far the misses are off.
 * @returns accuracyWithinOne — Share of predictions landing on the true level or one of its direct neighbours
 * @returns kendallTauB — Tie-corrected rank association: +1.0 orders the rows exactly as the truth does, 0.0 no association, -1.0 exactly backwards. This answers "does the model rank the rows correctly", which is a different question from "does it land on the right level" — a model whose every prediction is one level too high ranks perfectly and scores 1.0 here while the kappas drop. Consult it when the output feeds a sort, a triage queue or a threshold you can recalibrate, and read it against kappa to tell a miscalibrated model from a model that has learned nothing.
 * @returns spearmanRankCorrelation — The same ordering question as tau-b, computed as a correlation on midranks. It is the less conservative of the two under the heavy ties ordinal data always has, so it reads higher than tau-b on the same predictions; prefer tau-b when you need a defensible figure and this one when comparing against Spearman values reported elsewhere. Like tau-b it ignores calibration entirely.
 * @returns nSamples — Number of rows evaluated
 * @returns nLevels — Number of distinct levels both columns were ranked against
 * @returns result — All ordinal metrics plus the resolved level order they were computed against
 * @impure has side effects / drives control flow
 */
declare function mlOrdinalMetrics({ database: Struct, predictionsCol?: string, actualsCol?: string, classOrder?: string }): { quadraticWeightedKappa: float, linearWeightedKappa: float, meanAbsoluteRankError: float, macroMeanAbsoluteError: float, accuracyExact: float, accuracyWithinOne: float, kendallTauB: float, spearmanRankCorrelation: float, nSamples: int, nLevels: int, result: Struct };


// === AI/ML/Preprocessing ===

/**
 * Learn per-feature offsets and scales from a training table. Distance- and gradient-based models (Logistic Regression, Elastic Net, SVM, KNN, Gaussian Mixture) only behave when their features share a scale.
 * @param source (optional) — Choose which backend supplies the training data
 * @param method (optional) — Standard centers each feature and divides it by its standard deviation. MinMax squeezes each feature into the Min..Max range. MaxAbs divides each feature by its largest absolute value, keeping zeros at zero.
 * @param min (optional) — Lower bound of the target range. Only read when Method is MinMax.
 * @param max (optional) — Upper bound of the target range. Only read when Method is MinMax.
 * @returns model — Thread-safe handle to the fitted scaler. Feed it to Apply Transform to scale any table with these statistics.
 * @returns offsets — Value subtracted from each feature before scaling: the mean for Standard, the minimum for MinMax, zero for MaxAbs
 * @returns scales — Multiplier applied to each feature. linfa stores the reciprocal, so this is 1/std for Standard and 1/(max-min) for MinMax, and it stays 1 for constant features.
 * @impure has side effects / drives control flow
 */
declare function fitFeatureScaler({ source?: string, method?: string, min?: float, max?: float }): { model: Struct, offsets: float[], scales: float[] };

/**
 * Learn a vocabulary from a text column and turn documents into numeric vectors weighted by term frequency times inverse document frequency. Feed the fitted vectorizer to Apply Transform to vectorize a column, then train a classifier such as Multinomial Naive Bayes on the result. Tokenization always uses the built-in regex tokenizer, because a custom tokenizer function cannot be persisted and would make the saved model unloadable.
 * @param source (optional) — Choose which backend supplies the documents
 * @param method (optional) — Weighting formula. Smooth: log((1+n)/(1+df))+1, never divides by zero. Non-Smooth: log(n/df)+1, sharper but requires every term to appear at least once. Textbook: log(n/(1+df)), which discounts terms appearing in nearly every document down to a negative weight, so it cannot feed Multinomial Naive Bayes.
 * @param nGramMin (optional) — Smallest number of adjacent tokens forming a vocabulary entry (1 = single words)
 * @param nGramMax (optional) — Largest number of adjacent tokens forming a vocabulary entry. Must not be smaller than Min N-Gram.
 * @param convertToLowercase (optional) — Lowercase every document before tokenizing, so casing variants collapse into one vocabulary entry
 * @param maxFeatures (optional) — Keep only the most frequent N vocabulary entries, which caps the width of the produced vectors. 0 keeps all of them.
 * @param minDocumentFrequency (optional) — Drop terms appearing in a smaller share of documents than this (0-1). Useful to remove typos and one-off tokens.
 * @param maxDocumentFrequency (optional) — Drop terms appearing in a larger share of documents than this (0-1). Useful to remove boilerplate that carries no signal.
 * @param stopwords (optional) — Comma separated words to exclude from the vocabulary, e.g. `the, and, of`. Leave empty to keep every term.
 * @returns model — Thread-safe handle to the fitted TF-IDF vectorizer, for use with Apply Transform
 * @returns vocabulary — Learned vocabulary entries, in the same order as the columns of the produced vectors
 * @impure has side effects / drives control flow
 */
declare function fitTfidfVectorizer({ source?: string, method?: string, nGramMin?: int, nGramMax?: int, convertToLowercase?: bool, maxFeatures?: int, minDocumentFrequency?: float, maxDocumentFrequency?: float, stopwords?: string }): { model: Struct, vocabulary: string[] };

/**
 * Apply a fitted transformer (Feature Scaler, TF-IDF) to a table, writing one vector per row. A Feature Scaler replays the exact offsets and scales learned at fit time, so applying it to train and test gives both the same statistics. TF-IDF is different: linfa recomputes the inverse document frequencies from the table being transformed, so vectors are only comparable within a single Apply Transform run.
 * @param model — Fitted transformer to apply. Classifiers and regressors belong on the Predict node.
 * @param source (optional) — Choose which backend supplies the rows to transform
 * @param batchSize (optional) — Number of records to transform per batch (default: 5000, 0 = process all at once)
 * @impure has side effects / drives control flow
 */
declare function mlApplyTransform({ model: Struct, source?: string, batchSize?: int }): void;


// === AI/ML/Reduction ===

/**
 * Principal Component Analysis for dimensionality reduction
 * @param nComponents (optional) — Number of principal components to keep
 * @param source (optional) — Choose which backend supplies the data
 * @returns explainedVariance — Variance explained by each principal component
 * @impure has side effects / drives control flow
 */
declare function fitPca({ nComponents?: int, source?: string }): float[];

/**
 * t-Distributed Stochastic Neighbor Embedding. Projects high-dimensional vectors into 2-3 dimensions for visualization and writes the embedding back into the source table. t-SNE is transductive, so it produces no reusable model.
 * @param source (optional) — Choose which backend supplies the data
 * @param embeddingSize (optional) — Dimensionality of the embedding. Must not exceed the width of the input vectors; values above 3 require the exact gradient (Approx Threshold = 0).
 * @param perplexity (optional) — Effective number of neighbors per point (typically 5-50). t-SNE requires 3 * perplexity <= rows - 1, so small tables need a small perplexity.
 * @param approxThreshold (optional) — Barnes-Hut theta. 0 runs the exact O(n^2) gradient, larger values approximate distant points by their cell centroid and run faster.
 * @param maxIter (optional) — Number of gradient descent iterations. Fewer iterations finish sooner but may leave the embedding unconverged.
 * @impure has side effects / drives control flow
 */
declare function fitTsne({ source?: string, embeddingSize?: int, perplexity?: float, approxThreshold?: float, maxIter?: int }): void;


// === AI/ML/Regression ===

/**
 * Fit/Train a penalized linear regression model. Ridge shrinks all coefficients, Lasso drives irrelevant ones to exactly zero (feature selection), Elastic Net mixes both.
 * @param source (optional) — Choose which backend supplies the training data
 * @param penaltyType (optional) — Ridge = pure L2 (keeps all features, handles correlated ones well), Lasso = pure L1 (zeroes out weak features), ElasticNet = a blend controlled by L1 Ratio
 * @param penalty (optional) — Overall regularization strength. 0 means ordinary least squares, larger values shrink the coefficients harder.
 * @param l1Ratio (optional) — Share of the penalty spent on L1 vs L2. Only used when Penalty Type is ElasticNet; Ridge forces 0.0 and Lasso forces 1.0.
 * @param withIntercept (optional) — Fit a bias term. Disable only when the data is already centered.
 * @param maxIterations (optional) — Upper bound on coordinate descent passes. The solver stops silently at this cap, so a convergence warning is logged when it is hit.
 * @param tolerance (optional) — Convergence tolerance for coordinate descent. Smaller values give a tighter fit at the cost of more iterations.
 * @returns model — Thread-safe handle to the trained penalized regression model
 * @returns coefficients — Fitted coefficients and intercept. With Lasso, coefficients that are exactly zero mark features the model discarded.
 * @impure has side effects / drives control flow
 */
declare function fitElasticNet({ source?: string, penaltyType?: string, penalty?: float, l1Ratio?: float, withIntercept?: bool, maxIterations?: int, tolerance?: float }): { model: Struct, coefficients: Struct };

/**
 * Fit/Train a Generalized Linear Model. Pick the distribution that matches the target: Normal for unbounded values, Poisson for counts, Gamma for positive skewed amounts, Inverse Gaussian for heavy tails.
 * @param source (optional) — Choose which backend supplies the training data
 * @param distribution (optional) — Target distribution: Normal (power 0, any value), Poisson (power 1, counts >= 0), Gamma (power 2, values > 0), Inverse Gaussian (power 3, values > 0), or Custom to set the Tweedie power directly
 * @param power (optional) — Free Tweedie power, only used when Distribution is Custom. Values in (0, 1) do not describe any distribution and are rejected; (1, 2) is compound Poisson-Gamma.
 * @param alpha (optional) — Strength of the L2 penalty on the coefficients. 0 fits an unpenalized GLM.
 * @param fitIntercept (optional) — Fit a bias term. Disable only when the data is already centered.
 * @param maxIter (optional) — Iteration cap for the L-BFGS solver. Defaults to 1000 instead of the library default of 100, which is too low to converge on unscaled real-world features.
 * @param tol (optional) — Gradient tolerance that stops the L-BFGS solver. Smaller values fit tighter but need more iterations.
 * @returns model — Thread-safe handle to the trained generalized linear model
 * @impure has side effects / drives control flow
 */
declare function fitGlm({ source?: string, distribution?: string, power?: float, alpha?: float, fitIntercept?: bool, maxIter?: int, tol?: float }): Struct;

/**
 * Fit a K-Nearest-Neighbours regressor that averages the target of the nearest training rows. Non-parametric and instance based: the fitted model embeds a verbatim copy of the whole training set instead of learned coefficients, so every training row (and any personal data in it) travels with the model, is written into every saved model file and can be reconstructed by anyone holding it. Treat the model with the same care as the source table.
 * @param source (optional) — Choose which backend supplies the training data
 * @param k (optional) — How many nearest training rows are averaged for each prediction. Must be at least 1 and cannot exceed the number of training rows. Larger values smooth the response.
 * @param distanceWeighted (optional) — Weight each neighbour by the inverse of its distance instead of taking a plain mean. Reduces the pull of distant neighbours when k is large.
 * @returns model — Thread-safe handle to the trained KNN regressor. Contains a full copy of the training set.
 * @impure has side effects / drives control flow
 */
declare function fitKnnRegressor({ source?: string, k?: int, distanceWeighted?: bool }): Struct;

/**
 * Fit/Train Linear Regression Model
 * @param source (optional) — Choose where training data should be loaded from
 * @returns model — Thread-safe handle to the trained linear regression model
 * @impure has side effects / drives control flow
 */
declare function fitLinearRegression({ source?: string }): Struct;

/**
 * Fit/Train a Support Vector Regressor. Learns non-linear targets through a kernel, with epsilon-SVR or nu-SVR.
 * @param source (optional) — Choose which backend supplies the training data
 * @param mode (optional) — Epsilon-SVR penalises deviations larger than Epsilon. Nu-SVR replaces Epsilon with Nu, the target fraction of support vectors.
 * @param kernel (optional) — Feature-space mapping. Gaussian for smooth non-linear targets, Linear for the plain SVR, Polynomial for interaction terms.
 * @param kernelParam (optional) — Gaussian: the eps in exp(-||x - x'||^2 / eps), larger means smoother. Polynomial: the degree of (<x, x'> + 1)^degree. Ignored for Linear.
 * @param c (optional) — Penalty for deviations outside the tolerated margin. Higher values fit the training data harder and risk overfitting. Used by both modes.
 * @param epsilon (optional) — Width of the insensitive tube: errors smaller than this are not penalised. Epsilon-SVR only.
 * @param nu (optional) — Upper bound on the fraction of training errors and lower bound on the fraction of support vectors, in (0, 1]. Nu-SVR only.
 * @param tolerance (optional) — Stopping threshold of the SMO solver. Smaller values train longer for a more precise solution.
 * @returns model — Thread-safe handle to the trained support vector regressor
 * @returns supportVectors — Number of training rows that ended up contributing to the regression
 * @impure has side effects / drives control flow
 */
declare function fitSvmRegression({ source?: string, mode?: string, kernel?: string, kernelParam?: float, c?: float, epsilon?: float, nu?: float, tolerance?: float }): { model: Struct, supportVectors: int };


// === AI/ML/Teachable Machine ===

/**
 * Extract score from predictions.
 * @param prediction — Single ClassPrediction
 * @returns score — Selected prediction score
 */
declare function aiMlPredScore({ prediction: Struct }): float;


// === AI/ML/Tuning ===

/**
 * Automatically finds the best classification model. Cross-validates Naive Bayes, Decision Tree, Logistic Regression, Random Forest and SVM, then retrains the winner on the full dataset. The reported Best Model Type can be fed straight into Grid Search to tune it further.
 * @param cvFolds (optional) — Number of cross-validation folds
 * @param metric (optional) — Metric the leaderboard is ranked by. Accuracy is the share of correct rows; Macro F1 averages per-class F1 with equal weight per class, which is the right choice when the classes are imbalanced.
 * @param includeSvm (optional) — Include SVM in comparison (slower but often more accurate)
 * @param includeLogistic (optional) — Include Logistic Regression. Fast, and the only candidate that yields calibrated probabilities, but it expects scaled features — fit a Feature Scaler first for a fair comparison.
 * @param includeRandomForest (optional) — Include Random Forest. Usually the strongest candidate here, at the cost of training one tree per ensemble member on every fold.
 * @param source (optional) — Data source type
 * @returns results — Complete AutoML results with leaderboard
 * @returns bestModel — The best model trained on full data
 * @returns bestModelType — Name of the best algorithm
 * @impure has side effects / drives control flow
 */
declare function aiMlTuningAutoClassifier({ cvFolds?: int, metric?: string, includeSvm?: bool, includeLogistic?: bool, includeRandomForest?: bool, source?: string }): { results: Struct, bestModel: Struct, bestModelType: string };

/**
 * Automatically finds the best model for a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). Cross-validates the ordinal families - Proportional Odds and Ordered Probit, the all-threshold model and its support-vector form, Ordinal Ridge, Continuation Ratio and Adjacent Category, plus an optional rank-consistent neural family that is off by default because it costs far more than all the others combined - on identical folds, ranks them by an ordinal metric that knows how far a miss was, then retrains the winner on the full data. Use this rather than Auto Classifier, which resolves the target without its order and ranks by accuracy or macro-F1, scoring a five-level miss exactly like a one-level one. Every candidate here is a gradient or a least-squares fit on the raw columns, so scale your features with the Fit Feature Scaler node first: unscaled columns change which family wins, not just how fast it converges.
 * @param source (optional) — Choose which backend supplies the training data
 * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so the search fails rather than guessing one. The level set is resolved once here and handed to every family, so a level that a fold happens to miss cannot renumber the ranks for that fold.
 * @param cvFolds (optional) — How many folds the rows are split into. Every family is scored on the SAME folds, so the comparison is paired rather than a race between different splits. More folds mean a less noisy score and proportionally more fitting, since the whole sweep is repeated once per fold.
 * @param metric (optional) — What the leaderboard is ranked by. Quadratic Kappa is chance-corrected agreement that forgives a near miss and punishes a distant one four times as hard - the standard headline metric for ordered targets. Linear Kappa charges the same for every step along the scale, which is what you want when one level is one unit of loss. Mean Rank Error is the average number of levels a prediction is off by, and Macro Rank Error is the same averaged per true level so a rare level counts as much as the majority one - both are ERROR metrics, so the leaderboard ranks their smallest value first. Kendall Tau-b and Spearman ask only whether the rows come out in the right order and ignore calibration entirely, so a model whose levels are all shifted by one still scores perfectly.
 * @param seed (optional) — Seed for the fold shuffle. The same seed reproduces the same folds and therefore the same leaderboard; change it to check whether a narrow win survives a different split.
 * @param includeProportionalOdds (optional) — Try the cumulative-link model under a logit and a probit link. The only family here that yields calibrated per-level probabilities and coefficients that read as a direction along the ordering, but it assumes one shared effect across all cut points.
 * @param includeAllThreshold (optional) — Try the all-threshold model under a logistic and a hinge margin. It drops the proportional-odds assumption by fitting cut-point placement instead of a likelihood, which is often more robust when that assumption fails; the hinge entry is support vector ordinal regression. Neither yields per-level probabilities.
 * @param includeOrdinalRidge (optional) — Try rank regression with learned cut points across a small L2 sweep. Closed-form, so it is by far the cheapest candidate and stays cheap as levels and features grow - but it treats the ranks as numbers, so it is the family most likely to be beaten when the levels are not evenly spaced.
 * @param includeContinuationRatio (optional) — Try the sequential model, `P(stop at level k | reached level k)`. The right shape when reaching a level genuinely requires passing the ones below it (stages, escalation, dropout). It fits K-1 sub-models on shrinking subsets and refuses to fit at all when a middle level is missing from a fold, in which case it is dropped from the leaderboard and the other families continue.
 * @param includeAdjacentCategory (optional) — Try the adjacent-category model, which contrasts neighbouring levels instead of splitting the scale cumulatively. Reach for it when the interesting comparison is `this level versus the next one` rather than `at most this level versus above it`.
 * @param includeNeural (optional) — Try a small neural network under a rank-consistent head, as two candidates: a CORAL head, which shares one latent score across the cut points and lets them differ only by biases that cannot cross, and a CORN head, which fits one conditional task per cut point on the rows that reached it. OFF by default, unlike every other family here, and the default is the recommendation: a network is orders of magnitude more expensive to fit than the linear families, it is refitted from scratch on EVERY fold, and it is the one candidate that can dominate the runtime of the whole sweep. Switch it on when you suspect the levels are not separated by a single monotone direction in the features - the hidden layer is the entire contribution, and it is the only thing here that can represent such a boundary at all. On a problem that is linear in the features it can only match the simpler families, never beat them: with no hidden layer CORAL is EXACTLY the all-threshold model with a logistic margin and CORN is EXACTLY Continuation Ratio, so prefer those better-tested candidates when they win. Both use a fixed initialization seed, so the leaderboard stays reproducible. CORN is dropped from the leaderboard on any fold that omits a level nothing reaches, since its task for that level would have no rows; CORAL has no such failure mode.
 * @returns results — Leaderboard of every configuration that finished, best first, plus the ones that were dropped and why. `higher_is_better` states which end of `cv_score` won.
 * @returns bestModel — The winning configuration retrained on the full dataset. Predictions come back as your original level labels.
 * @returns bestModelType — Model kind of the winner, e.g. `OrdinalLogistic`. Read back off the retrained model, so it always matches what the rest of the catalog calls it.
 * @returns levels — The level order every candidate was trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when the leaderboard looks upside down.
 * @impure has side effects / drives control flow
 */
declare function aiMlTuningAutoOrdinal({ source?: string, classOrder?: string, cvFolds?: int, metric?: string, seed?: int, includeProportionalOdds?: bool, includeAllThreshold?: bool, includeOrdinalRidge?: bool, includeContinuationRatio?: bool, includeAdjacentCategory?: bool, includeNeural?: bool }): { results: Struct, bestModel: Struct, bestModelType: string, levels: Struct };

/**
 * Exhaustive search over parameter combinations with cross-validation. Returns the best parameters found. Model Type accepts the same names the Auto Classifier reports as its best model, so the two nodes chain directly.
 * @param modelType (optional) — Type of model to tune
 * @param cvFolds (optional) — Number of cross-validation folds
 * @param source (optional) — Database containing the training data
 * @returns results — Complete grid search results with all combinations tried
 * @returns bestModel — The model trained with the best parameters on full training data
 * @impure has side effects / drives control flow
 */
declare function aiMlTuningGridSearch({ modelType?: string, cvFolds?: int, source?: string }): { results: Struct, bestModel: Struct };

/**
 * Exhaustively searches the hyperparameters of ONE ordinal model family with cross-validation, for a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). Every combination in the Parameter Grid is scored on the SAME folds and ranked by an ordinal metric that knows how far a miss was. Use this rather than Grid Search, which resolves the target without its order and tunes against accuracy, scoring a five-level miss exactly like a one-level one. Model Type accepts the names Auto Ordinal reports as its best model, so the usual chain is Auto Ordinal to pick the family, then this node to tune it. Every family here is a gradient or a least-squares fit on the raw columns, so scale your features with the Fit Feature Scaler node first: unscaled columns change which hyperparameters win, not just how fast they converge.
 * @param source (optional) — Choose which backend supplies the training data
 * @param modelType (optional) — Which ordinal family to tune. OrdinalLogistic is the threshold model, the widest family here: it takes a link, a loss and a margin, and covers proportional odds, ordered probit and support vector ordinal regression. OrdinalRidge is rank regression with learned cut points, closed-form and so by far the cheapest to sweep, but it has only a penalty to tune. OrdinalContinuationRatio models a sequential progression, `P(stop at k | reached k)`. OrdinalAdjacentCategory contrasts neighbouring levels instead of splitting the scale cumulatively. OrdinalNeural is a small network under a rank-consistent CORAL or CORN head, the only family here that is not linear in the features and the only one that can represent a level that is not monotone in them - and by a wide margin the most expensive to sweep, since every combination trains a whole network from scratch on every fold, so keep its grid small. Switching this after the Parameter Grid was seeded does NOT rewrite the grid - the run rejects parameters the new family does not consume rather than ignoring them silently.
 * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so the search fails rather than guessing one. The level set is resolved once here and handed to every fit, so a level that a fold happens to miss cannot renumber the ranks for that fold.
 * @param cvFolds (optional) — How many folds the rows are split into. Every combination is scored on the SAME folds, so the comparison is paired rather than a race between different splits. More folds mean a less noisy score and proportionally more fitting, since the whole grid is refitted once per fold.
 * @param metric (optional) — What the sweep is ranked by. Quadratic Kappa is chance-corrected agreement that forgives a near miss and punishes a distant one four times as hard - the standard headline metric for ordered targets. Linear Kappa charges the same for every step along the scale, which is what you want when one level is one unit of loss. Mean Rank Error is the average number of levels a prediction is off by, and Macro Rank Error is the same averaged per true level so a rare level counts as much as the majority one - both are ERROR metrics, so the SMALLEST value wins and the `higher_is_better` output says so. Kendall Tau-b and Spearman ask only whether the rows come out in the right order and ignore calibration entirely, so a model whose levels are all shifted by one still scores perfectly.
 * @param seed (optional) — Seed for the fold shuffle, and for the weight initialization when Model Type is OrdinalNeural - the two sources of randomness in the sweep, tied to one value so the same seed reproduces the same folds, the same fits and therefore the same winner. Change it to check whether a narrow win survives a different split, which for the neural family also re-rolls the starting point of a non-convex fit. The winner is retrained from the same initialization it was scored at.
 * @returns results — Every combination that completed all folds with its mean and spread across the folds, plus the ones that were dropped and why. `higher_is_better` states which end of `mean_score` won.
 * @returns bestModel — The winning combination retrained on the full dataset. Predictions come back as your original level labels.
 * @returns bestScore — Mean cross-validated score of the winner, in the units of the chosen metric. Meaningless without Higher Is Better: for the two error metrics this is the SMALLEST score in the sweep, not the largest.
 * @returns higherIsBetter — Direction of the chosen metric: true for the agreement measures, false for MeanAbsoluteRankError and MacroMeanAbsoluteError, where a smaller score is the better model. Branch on this rather than assuming, otherwise a comparison downstream will rank the sweep upside down.
 * @returns levels — The level order every configuration was trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when the results look upside down.
 * @impure has side effects / drives control flow
 */
declare function aiMlTuningOrdinalGridSearch({ source?: string, modelType?: string, classOrder?: string, cvFolds?: int, metric?: string, seed?: int }): { results: Struct, bestModel: Struct, bestScore: float, higherIsBetter: bool, levels: Struct };


// === AI/Memory ===

/**
 * Assembles retrieved memory records into a token-budgeted context string for injection into agent system prompts
 * @param memoryConfig — MemoryConfig for token budget
 * @param memories — Array of memory records from Search Memory node
 * @param header (optional) — Optional header text prepended to the context block
 * @returns contextText — Assembled memory context string, ready for system prompt injection
 * @returns tokenEstimate — Approximate token count of the assembled context
 * @impure has side effects / drives control flow
 */
declare function memoryBuildContext({ memoryConfig: Struct, memories: Struct[], header?: string }): { contextText: string, tokenEstimate: int };

/**
 * Compresses old memory observations into a summary using an LLM, then replaces them in the store. Runs the embedding model to store the summary vector.
 * @param memoryConfig — MemoryConfig from Create Memory Config node
 * @param observations — Array of memory records to compress (typically older observations from Search Memory)
 * @param model — LLM model Bit for generating the summary
 * @returns summaryText — The compressed summary text
 * @returns compressedCount — Number of observations that were compressed
 * @returns stats — Token usage and model statistics from the compaction LLM call
 * @impure has side effects / drives control flow
 */
declare function memoryCompress({ memoryConfig: Struct, observations: Struct[], model: Struct }): { summaryText: string, compressedCount: int, stats: Struct };

/**
 * Creates a MemoryConfig that bundles database, embedding model, and tuning parameters for all memory nodes
 * @param database — LanceDB connection (from Open Database node). The table IS the scope boundary — use one table per user/session.
 * @param embeddingModel — Cached embedding model for vector search (from Load Embedding Model node)
 * @param maxContextTokens (optional) — Token budget for assembled memory context
 * @param recallStrategy (optional) — How to retrieve memories: recent_first (last N), relevance (vector similarity), hybrid (both)
 * @param recallTopK (optional) — Max items returned from vector search
 * @param autoCompress (optional) — Automatically compress old observations when threshold is reached
 * @param compressThreshold (optional) — Number of observations before triggering compression
 * @returns memoryConfig — Configured MemoryConfig — pass to any memory node
 */
declare function memoryCreateConfig({ database: Struct, embeddingModel: Struct, maxContextTokens?: int, recallStrategy?: string, recallTopK?: int, autoCompress?: bool, compressThreshold?: int }): Struct;

/**
 * Runs LanceDB maintenance on the memory table: flush buffered writes, compact fragments, prune old versions, and rebuild indices. Run periodically or after bulk writes.
 * @param memoryConfig — MemoryConfig from Create Memory Config node
 * @param keepVersions (optional) — Whether to keep old row versions (false = prune for disk savings)
 * @impure has side effects / drives control flow
 */
declare function memoryOptimize({ memoryConfig: Struct, keepVersions?: bool }): void;

/**
 * Searches the memory store using the configured recall strategy (recent, relevance, or hybrid)
 * @param memoryConfig — MemoryConfig from Create Memory Config node
 * @param query — Search query text — used for vector similarity and/or full-text search
 * @param roleFilter (optional) — Optional role filter (one of: user, assistant, observation, summary, context)
 * @returns results — Array of matching memory records (sorted by relevance/recency)
 * @returns resultCount — Number of results returned
 * @impure has side effects / drives control flow
 */
declare function memorySearch({ memoryConfig: Struct, query: string, roleFilter?: string }): { results: Struct[], resultCount: int };

/**
 * Embeds text and stores it as a memory observation in the configured LanceDB table
 * @param memoryConfig — MemoryConfig from Create Memory Config node
 * @param content — Text content to store as a memory observation
 * @param role (optional) — Role of the message author
 * @returns observationCount — Total number of observations in the memory table after this insert
 * @impure has side effects / drives control flow
 */
declare function memoryStore({ memoryConfig: Struct, content: string, role?: string }): int;


// === AI/Memory/Graph ===

/**
 * Extracts entities (nodes) and relationships (edges) from text using an LLM, returning structured arrays ready for graph insertion
 * @param graph — Graph connection from Open Graph Overlay node
 * @param text — Input text to extract entities and relationships from
 * @param nodeLabels — Allowed node labels for extraction (from overlay definition)
 * @param edgeLabels — Allowed edge labels for extraction (from overlay definition)
 * @returns errorMessage — Error details
 * @returns extractedNodes — Array of extracted entity objects with label, id, and properties
 * @returns extractedEdges — Array of extracted relationship objects with label, source, target, and properties
 * @returns entityCount — Total number of entities extracted
 * @impure has side effects / drives control flow
 */
declare function kgExtract({ graph: Struct, text: string, nodeLabels: string[], edgeLabels: string[] }): { errorMessage: string, extractedNodes: Struct[], extractedEdges: Struct[], entityCount: int };

/**
 * Retrieves context from a knowledge graph: embeds the query, finds matching nodes, then expands N hops to build structured context
 * @param graph — Graph connection from Open Graph Overlay node
 * @param query — Natural language query to search for in the graph
 * @param nodeLabel — Label of the node type to search (must have a vector column)
 * @param depth (optional) — Number of hops to expand from matched nodes
 * @param topK (optional) — Number of seed nodes to retrieve via embedding search
 * @param limit (optional) — Maximum total nodes + edges in the expanded subgraph
 * @returns errorMessage — Error details
 * @returns context — Structured subgraph context as JSON (nodes + edges + properties)
 * @returns summaryText — Flattened text representation of the retrieved subgraph for LLM consumption
 * @returns nodeCount — Number of nodes in the result
 * @impure has side effects / drives control flow
 */
declare function kgRetrieve({ graph: Struct, query: string, nodeLabel: string, depth?: int, topK?: int, limit?: int }): { errorMessage: string, context: Struct, summaryText: string, nodeCount: int };

/**
 * Converts a subgraph (nodes + edges) into a natural-language summary for LLM consumption
 * @param graph — Graph connection reference (for label metadata)
 * @param subgraph — Subgraph payload (output from KG Retrieve, Neighbors, or Subgraph nodes)
 * @param maxTokens (optional) — Approximate maximum token budget for the summary (controls verbosity)
 * @param includeProperties (optional) — Whether to include node/edge properties in the summary
 * @returns summary — Natural-language summary of the subgraph
 * @returns nodeCount — Number of nodes in the input subgraph
 * @returns edgeCount — Number of edges in the input subgraph
 * @impure has side effects / drives control flow
 */
declare function kgSummarize({ graph: Struct, subgraph: Struct, maxTokens?: int, includeProperties?: bool }): { summary: string, nodeCount: int, edgeCount: int };


// === AI/Preprocessing ===

/**
 * Splits long text into sized/overlapping chunks using the cached embedding model's splitter
 * @param text — Source string that needs chunking
 * @param model — Cached embedding Bit providing the tokenizer/splitter
 * @param capacity (optional) — Max characters/tokens in each chunk
 * @param overlap (optional) — How many characters/tokens overlap between consecutive chunks
 * @param markdown (optional) — Use a Markdown-aware splitter (true) or the plain splitter
 * @returns chunks — Array of chunked text segments
 * @impure has side effects / drives control flow
 */
declare function chunkText({ text: string, model: Struct, capacity?: int, overlap?: int, markdown?: bool }): string[];

/**
 * Splits raw text locally using simple character-based chunking
 * @param text — Source string that should be chunked
 * @param capacity (optional) — Maximum characters per chunk
 * @param overlap (optional) — Character overlap between adjacent chunks
 * @param markdown (optional) — Use Markdown-aware splitting (true) or basic splitter
 * @returns chunks — Character chunk array
 * @impure has side effects / drives control flow
 */
declare function chunkTextChar({ text: string, capacity?: int, overlap?: int, markdown?: bool }): string[];


// === AI/Processing ===

/**
 * Extracts keywords from text using an LLM. The AI understands context and semantics, providing high-quality keyword extraction for complex or domain-specific content.
 * @param model — LLM to use for keyword extraction
 * @param text (optional) — The text to extract keywords from
 * @param maxKeywords (optional) — Maximum number of keywords to extract
 * @param context (optional) — Optional context or instructions for keyword extraction (e.g., 'focus on technical terms' or 'extract product names')
 * @returns keywords — Extracted keywords as a string set
 * @impure has side effects / drives control flow
 */
declare function aiProcessingAiKeywordExtraction({ model: Struct, text?: string, maxKeywords?: int, context?: string }): Set<string>;

/**
 * Intelligently segments document into thematic sections with summaries, tracking content across non-contiguous pages. Ideal for large document corpora.
 * @param pages — Document pages to segment into sections.
 * @param model — AI model for semantic analysis.
 * @param maxContextTokens (optional) — Maximum tokens per analysis chunk.
 * @param parallelRequests (optional) — Number of chunks to process in parallel. Set to 0 or chunks count to process all at once.
 * @returns sections — Array of thematic content sections with cross-page tracking.
 * @impure has side effects / drives control flow
 */
declare function aiProcessingExtractContentSections({ pages: Struct[], model: Struct, maxContextTokens?: int, parallelRequests?: int }): Struct[];

/**
 * Extracts text and content from documents (PDF, DOCX, XLSX, images, etc.) and converts to markdown.
 * @param file — Document file to extract (PDF, DOCX, XLSX, images, etc.).
 * @param extractImages (optional) — Whether to extract and embed images from the document.
 * @returns pages — Extracted document pages with content and images.
 * @impure has side effects / drives control flow
 */
declare function aiProcessingExtractDocument({ file: Struct, extractImages?: bool }): Struct[];

/**
 * Extracts text and content from documents using AI for enhanced image descriptions and OCR.
 * @param file — Document file to extract (PDF, DOCX, XLSX, images, etc.).
 * @param model — Vision-capable AI model for image analysis and OCR.
 * @param extractImages (optional) — Whether to extract and embed images from the document.
 * @param imagesPerMessage (optional) — Number of images to batch per LLM request (higher = faster but may hit token limits).
 * @param pagesPerBatch (optional) — Number of PDF pages to process in parallel (higher = faster but uses more memory).
 * @param temperature (optional) — LLM temperature (0.0 = deterministic, 1.0 = creative). Lower is better for extraction.
 * @param maxTokens (optional) — Maximum output tokens per LLM call. Leave at 0 for model default. Set lower for unreliable models.
 * @returns pages — Extracted document pages with AI-generated descriptions and images.
 * @impure has side effects / drives control flow
 */
declare function aiProcessingExtractDocumentAi({ file: Struct, model: Struct, extractImages?: bool, imagesPerMessage?: int, pagesPerBatch?: int, temperature?: float, maxTokens?: int }): Struct[];

/**
 * Extracts text and content from multiple documents in parallel.
 * @param files — Array of document files to extract.
 * @param extractImages (optional) — Whether to extract and embed images from documents.
 * @returns results — Array of extracted document pages for each file.
 * @impure has side effects / drives control flow
 */
declare function aiProcessingExtractDocuments({ files: Struct[], extractImages?: bool }): Struct[];

/**
 * Extracts text and content from multiple documents using AI in parallel.
 * @param files — Array of document files to extract.
 * @param model — Vision-capable AI model for image analysis and OCR.
 * @param extractImages (optional) — Whether to extract and embed images from documents.
 * @param imagesPerMessage (optional) — Number of images to batch per LLM request (higher = faster but may hit token limits).
 * @param pagesPerBatch (optional) — Number of PDF pages to process in parallel (higher = faster but uses more memory).
 * @param temperature (optional) — LLM temperature (0.0 = deterministic, 1.0 = creative). Lower is better for extraction.
 * @param maxTokens (optional) — Maximum output tokens per LLM call. Leave at 0 for model default. Set lower for unreliable models.
 * @returns results — Array of extracted document pages with AI descriptions for each file.
 * @impure has side effects / drives control flow
 */
declare function aiProcessingExtractDocumentsAi({ files: Struct[], model: Struct, extractImages?: bool, imagesPerMessage?: int, pagesPerBatch?: int, temperature?: float, maxTokens?: int }): Struct[];

/**
 * Combines an array of document pages into a single markdown string.
 * @param pages — Array of document pages to combine.
 * @returns markdown — Combined markdown content from all pages.
 * @impure has side effects / drives control flow
 */
declare function aiProcessingPagesToMarkdown({ pages: Struct[] }): string;

/**
 * Extracts keywords from text using the RAKE (Rapid Automatic Keyword Extraction) algorithm. RAKE is a domain-independent algorithm that extracts significant phrases by analyzing word frequency and co-occurrence.
 * @param text (optional) — The text to extract keywords from
 * @param language (optional) — Language for stop words. Use 'auto' for automatic detection.
 * @param stopWords (optional) — Custom stop words to filter out (optional). Overrides language-based stop words if provided.
 * @param minScore (optional) — Minimum score threshold for keywords (0.0 = no filter)
 * @param maxKeywords (optional) — Maximum number of keywords to return (0 = unlimited)
 * @returns keywords — Extracted keywords as a string set
 */
declare function aiProcessingRakeExtraction({ text?: string, language?: string, stopWords?: Set<string>, minScore?: float, maxKeywords?: int }): Set<string>;

/**
 * Creates an intelligent summary of document pages using AI with configurable strategies and detail levels. Handles long documents via chunked summarization with multiple strategy options.
 * @param pages — Document pages to summarize.
 * @param model — AI model to use for summarization.
 * @param detailLevel (optional) — Summary detail level: Low (very concise), Medium (balanced), High (comprehensive).
 * @param includeToc (optional) — Whether to include a table of contents with page references.
 * @param strategy (optional) — Summarization strategy: • Refine — sequential, best coherence, no parallelism • MapReduce — parallel chunking, fast, may lose cross-chunk context • Hierarchical — structure-aware tree, best for headed documents • Hybrid — MapReduce speed + Refine coherence polish • SlidingWindow — fixed memory buffer, best for very long documents
 * @param densification (optional) — Post-processing to increase information density: • None — use the strategy output as-is • ChainOfDensity — iteratively compress to optimal density
 * @param maxContextTokens (optional) — Maximum characters per summarization chunk (adjust based on model context window).
 * @param chunkOverlap (optional) — Overlap between adjacent chunks as percentage (0-50). Prevents information loss at boundaries (default: 10).
 * @param trackEntities (optional) — Extract and track named entities across chunks to prevent information loss.
 * @param parallelRequests (optional) — Number of chunks to process in parallel for MapReduce/Hybrid strategies. 0 = unlimited (default: 4).
 * @param densitySteps (optional) — Number of Chain of Density refinement steps when densification is enabled (1-5, default: 3).
 * @returns summary — The generated document summary.
 * @impure has side effects / drives control flow
 */
declare function aiProcessingSummarizeDocument({ pages: Struct[], model: Struct, detailLevel?: string, includeToc?: bool, strategy?: string, densification?: string, maxContextTokens?: int, chunkOverlap?: int, trackEntities?: bool, parallelRequests?: int, densitySteps?: int }): Struct;

/**
 * Extracts keywords from text using YAKE (Yet Another Keyword Extractor). YAKE is an unsupervised automatic keyword extraction method that uses statistical features from the text itself.
 * @param text (optional) — The text to extract keywords from
 * @param language (optional) — Language code for stop words. Use 'auto' for automatic detection.
 * @param ngrams (optional) — Maximum n-gram size (1-3). Higher values extract longer phrases.
 * @param maxKeywords (optional) — Maximum number of keywords to return
 * @param dedupThreshold (optional) — Levenshtein distance threshold for deduplication (0.0-1.0). Lower means stricter deduplication.
 * @returns keywords — Extracted keywords as a string set
 */
declare function aiProcessingYakeExtraction({ text?: string, language?: string, ngrams?: int, maxKeywords?: int, dedupThreshold?: float }): Set<string>;

/**
 * Masks Personally Identifiable Information using an LLM. Can detect contextual PII like names, addresses, and sensitive information that regex patterns might miss.
 * @param model — LLM to use for PII detection
 * @param text (optional) — The text to scan for PII
 * @param maskText (optional) — Text to replace PII with (default: [REDACTED])
 * @param additionalContext (optional) — Additional instructions for PII detection (e.g., 'focus on medical records' or 'mask company names')
 * @param sensitivity (optional) — Detection sensitivity level
 * @returns maskedText — Text with PII masked
 * @returns detectionCount — Number of PII instances detected and masked
 * @returns detections — Array with detection details (type, original value, context)
 * @impure has side effects / drives control flow
 */
declare function processingPiiMaskAi({ model: Struct, text?: string, maskText?: string, additionalContext?: string, sensitivity?: string }): { maskedText: string, detectionCount: int, detections: Struct[] };

