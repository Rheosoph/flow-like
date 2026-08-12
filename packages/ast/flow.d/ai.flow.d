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
 * Fit/Train a Decision Tree classifier. Native multi-class support with interpretable rules.
 * @param source (optional) — Choose which backend supplies the training data
 * @param maxDepth (optional) — Maximum depth of the tree. None means unlimited.
 * @param minSamplesSplit (optional) — Minimum number of samples required to split a node
 * @returns model — Thread-safe handle to the trained Decision Tree classifier
 * @impure has side effects / drives control flow
 */
declare function fitDecisionTree({ source?: string, maxDepth?: int, minSamplesSplit?: int }): Struct;

/**
 * Fit/Train a Gaussian Naive Bayes classifier. Native multi-class support - no need for One-vs-All.
 * @param source (optional) — Choose which backend supplies the training data
 * @returns model — Thread-safe handle to the trained Naive Bayes classifier
 * @impure has side effects / drives control flow
 */
declare function fitNaiveBayes({ source?: string }): Struct;

/**
 * Fit/Train Support Vector Machines (SVM) for Multi-Class Classification
 * @param source (optional) — Choose which backend supplies the training data
 * @returns model — Thread-safe handle to the trained SVM classifier
 * @impure has side effects / drives control flow
 */
declare function fitSvmMultiClass({ source?: string }): Struct;


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
 * Fit/Train KMeans Clustering
 * @param cluster (optional) — Choose how many centroids to fit
 * @param source (optional) — Choose which backend supplies the training data
 * @returns model — Thread-safe handle to the trained KMeans model
 * @impure has side effects / drives control flow
 */
declare function fitKmeans({ cluster?: int, source?: string }): Struct;


// === AI/ML/Dataset ===

/**
 * Generate K train/test splits for cross-validation. Each fold uses (K-1)/K data for training and 1/K for validation.
 * @param k (optional) — Number of folds for cross-validation (typically 5 or 10)
 * @param shuffle (optional) — Randomly shuffle data before splitting
 * @param source — Source database containing the dataset
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
 * Split a dataset into training and testing subsets while maintaining class distribution
 * @param split (optional) — Ratio used for assigning rows to the training set (rest goes to test)
 * @param labelColumn (optional) — Name of the column containing class labels for stratification
 * @param source — Data Source (DB or CSV)
 * @param train — Destination database connection that receives the training rows
 * @param test — Destination database connection that receives the testing rows
 * @impure has side effects / drives control flow
 */
declare function aiMlDatasetStratifiedSplit({ split?: float, labelColumn?: string, source: Struct, train: Struct, test: Struct }): void;


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


// === AI/ML/Model Info ===

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
 * Extract named entities (persons, organizations, locations, dates, etc.) from text using ONNX models. Supports BERT, RoBERTa, and other transformer-based NER models with automatic tokenization. Download models from: BERT-base-NER (https://huggingface.co/dslim/bert-base-NER), Multilingual NER (https://huggingface.co/Davlan/bert-base-multilingual-cased-ner-hrl), spaCy NER (https://huggingface.co/spacy). Download tokenizer.json from the same model repository.
 * @param model — ONNX NER Model Session
 * @param tokenizer — HuggingFace tokenizer.json file for BERT/RoBERTa tokenization. Download from the same model repository.
 * @param text — Input text to analyze for named entities
 * @param labels — Entity label names in model output order (e.g. ['O', 'B-PER', 'I-PER', 'B-ORG', ...]). If empty, uses CoNLL-2003 default.
 * @param scheme (optional) — Tagging scheme: BIO, BIOES, IOB, or BILOU
 * @param threshold (optional) — Minimum confidence threshold for entity extraction (0.0-1.0)
 * @param maxLength (optional) — Maximum sequence length for tokenization (default: 512)
 * @returns result — Full NER result with entities and token predictions
 * @returns entities — Extracted named entities as array
 * @returns entityCount — Number of entities found
 * @impure has side effects / drives control flow
 */
declare function onnxNer({ model: Struct, tokenizer: Struct, text: string, labels: string[], scheme?: Struct, threshold?: float, maxLength?: int }): { result: Struct, entities: Struct[], entityCount: int };


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
 * t-Distributed Stochastic Neighbor Embedding for dimensionality reduction (placeholder - not yet implemented)
 * @param nComponents (optional) — Number of dimensions to reduce to (typically 2 or 3)
 * @param perplexity (optional) — Related to the number of nearest neighbors (typical values: 5-50)
 * @impure has side effects / drives control flow
 */
declare function fitTsne({ nComponents?: int, perplexity?: float }): void;


// === AI/ML/Regression ===

/**
 * Fit/Train Linear Regression Model
 * @param source (optional) — Choose where training data should be loaded from
 * @returns model — Thread-safe handle to the trained linear regression model
 * @impure has side effects / drives control flow
 */
declare function fitLinearRegression({ source?: string }): Struct;


// === AI/ML/Teachable Machine ===

/**
 * Extract score from predictions.
 * @param prediction — Single ClassPrediction
 * @returns score — Selected prediction score
 */
declare function aiMlPredScore({ prediction: Struct }): float;


// === AI/ML/Tuning ===

/**
 * Automatically finds the best classification model. Tries Naive Bayes, Decision Tree, and SVM with cross-validation.
 * @param cvFolds (optional) — Number of cross-validation folds
 * @param metric (optional) — Optimization metric
 * @param includeSvm (optional) — Include SVM in comparison (slower but often more accurate)
 * @param source (optional) — Data source type
 * @returns results — Complete AutoML results with leaderboard
 * @returns bestModel — The best model trained on full data
 * @returns bestModelType — Name of the best algorithm
 * @impure has side effects / drives control flow
 */
declare function aiMlTuningAutoClassifier({ cvFolds?: int, metric?: string, includeSvm?: bool, source?: string }): { results: Struct, bestModel: Struct, bestModelType: string };

/**
 * Exhaustive search over parameter combinations with cross-validation. Returns the best parameters found.
 * @param modelType (optional) — Type of model to tune
 * @param cvFolds (optional) — Number of cross-validation folds
 * @param source (optional) — Database containing the training data
 * @returns results — Complete grid search results with all combinations tried
 * @returns bestModel — The model trained with the best parameters on full training data
 * @impure has side effects / drives control flow
 */
declare function aiMlTuningGridSearch({ modelType?: string, cvFolds?: int, source?: string }): { results: Struct, bestModel: Struct };


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

