// llm — FlowScript node declarations (generated, do not edit).
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
 * Prepares a Bit for MiniMax's OpenAI-compatible API using the provided credentials
 * @param endpoint (optional) — MiniMax OpenAI-compatible base URL (override only for a proxy)
 * @param apiKey (optional) — MiniMax API key used for authentication
 * @param modelId (optional) — MiniMax model identifier to request
 * @returns model — Bit containing the provider configuration
 * @impure has side effects / drives control flow
 */
declare function aiGenerativeBuildMinimax({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

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


// === Events/Chat ===

/**
 * Pulls down image, audio, video, and document attachments referenced in the latest chat message
 * @param history — Chat history whose final message may contain media parts
 * @param attachments (optional) — Existing attachments to merge with new downloads
 * @returns paths — Virtual file paths pointing to cached attachments
 * @impure has side effects / drives control flow
 */
declare function aiGenLlmHistoryExtractAttachments({ history: Struct, attachments?: Struct[] }): Struct[];

