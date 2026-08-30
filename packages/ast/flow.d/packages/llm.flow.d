// llm — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace agent {
    // === AI/Agents ===

    /**
     * Executes an Agent with history and returns the complete response
     * @node agent_invoke @receiver agent @alias agentInvoke
     * @param agent — Configured Agent object with tools (receiver: `this` in `x.invoke(...)`)
     * @param history — Conversation history to provide context
     * @returns response — Final agent response
     * @returns historyOut — Updated conversation history with agent turns
     * @returns stats — Token usage, cost, and model statistics
     * @impure has side effects / drives control flow
     */
    function invoke(this: Agent, { agent: Struct, history: Struct }): { response: Struct, historyOut: Struct, stats: Struct };

    /**
     * LLM-driven control loop that repeatedly calls referenced Flow functions as tools until it decides to stop
     * @node simple_agent @alias simpleAgent
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
    function simple({ model: Struct, history: Struct, maxIter?: int, infiniteContext?: bool, maxContextTokens?: int, contextMode?: string }): { chunk: Struct, response: Struct, historyOut: Struct, stats: Struct };

    /**
     * Executes an Agent with streaming, emitting chunks in real-time
     * @node agent_stream_invoke @receiver agent @alias agentStreamInvoke
     * @param agent — Configured Agent object with tools (receiver: `this` in `x.streamInvoke(...)`)
     * @param history — Conversation history to provide context
     * @returns chunk — Latest streamed chunk from agent response
     * @returns response — Final complete agent response
     * @returns historyOut — Updated conversation history with all agent turns
     * @returns stats — Token usage, cost, and model statistics
     * @impure has side effects / drives control flow
     */
    function streamInvoke(this: Agent, { agent: Struct, history: Struct }): { chunk: Struct, response: Struct, historyOut: Struct, stats: Struct };

    // === AI/Agents/Builder ===

    /**
     * Add a DataFusion SQL session to an agent for data analysis capabilities
     * @node add_datafusion_to_agent @receiver agent @alias addDatafusionToAgent
     * @param agent — Agent to add DataFusion context to (receiver: `this` in `x.addDatafusion(...)`)
     * @param session — DataFusion session from CreateDataFusionSession node
     * @param description — User-friendly description of this data source
     * @param tableDescriptions — Map of table names to descriptions (JSON object)
     * @param exampleQueries — Example SQL queries that work with this data
     * @param discoverSchemas (optional) — Automatically discover table schemas at runtime
     * @returns agentOut — Agent with DataFusion context added
     * @impure has side effects / drives control flow
     */
    function addDatafusion(this: Agent, { agent: Struct, session: Struct, description: string, tableDescriptions: Struct, exampleQueries: any, discoverSchemas?: bool }): Struct;

    /**
     * Creates an Agent object from a model Bit with configuration
     * @node agent_from_model @alias agentFromModel
     * @param model — LLM model Bit that will power the agent
     * @param maxIter (optional) — Maximum number of tool call iterations before stopping
     * @param infiniteContext (optional) — Enable automatic context window management to prevent overflow
     * @param contextMode (optional) — Strategy: 'truncate' (fast, drops old messages) or 'summarize' (LLM compresses history, slower but preserves info)
     * @param maxContextTokens (optional) — Maximum tokens to retain in context window (default: 32000)
     * @returns agentOut — Configured Agent object ready for tool registration and execution
     */
    function fromModel({ model: Struct, maxIter?: int, infiniteContext?: bool, contextMode?: string, maxContextTokens?: int }): Struct;

    /**
     * Indexes referenced Flow-Like functions into a vector DB so agents can discover tools via semantic search at runtime, keeping the context window lean.
     * @node agent_lazy_register_function_tools @receiver agent_in @alias agentLazyRegisterFunctionTools
     * @param agentIn — Agent object to register lazy function tools on (receiver: `this` in `x.lazyRegisterFunctionTools(...)`)
     * @param model — Embedding model used to index functions for semantic search
     * @returns agentOut — Agent with lazy function tool references attached
     * @impure has side effects / drives control flow
     */
    function lazyRegisterFunctionTools(this: Agent, { agentIn: Struct, model: Struct }): Struct;

    /**
     * Adds referenced Flow-Like functions as callable tool references to an Agent
     * @node agent_register_function_tools @receiver agent_in @alias agentRegisterFunctionTools
     * @param agentIn — Agent object to add function references to (receiver: `this` in `x.registerFunctionTools(...)`)
     * @returns agentOut — Agent object with registered function tool references
     */
    function registerFunctionTools(this: Agent, { agentIn: Struct }): Struct;

    /**
     * Registers a knowledge graph traversal tool on the agent so it can query the graph mid-conversation
     * @node kg_traverse_tool @receiver agent_in @alias kgTraverseTool
     * @param agentIn — Agent to register the KG tool on (receiver: `this` in `x.registerKgTraverseTool(...)`)
     * @param graph — Graph connection from Open Graph Overlay node
     * @param toolName (optional) — Name for the registered tool (shown to the LLM)
     * @param toolDescription (optional) — Description of the tool for the LLM
     * @returns agentOut — Agent with the KG traverse tool registered
     */
    function registerKgTraverseTool(this: Agent, { agentIn: Struct, graph: Struct, toolName?: string, toolDescription?: string }): Struct;

    /**
     * Adds Model Context Protocol (MCP) server tools to an Agent
     * @node agent_register_mcp_tools @receiver agent_in @alias agentRegisterMcpTools
     * @param agentIn — Agent object to add MCP tools to (receiver: `this` in `x.registerMcpTools(...)`)
     * @param uri — URI of the MCP server to connect to
     * @param mode (optional) — How to select MCP tools (Automatic = all tools, Manual = pick specific tools)
     * @returns agentOut — Agent object with registered MCP tools
     */
    function registerMcpTools(this: Agent, { agentIn: Struct, uri: string, mode?: string }): Struct;

    /**
     * Gives the agent autonomous access to persistent memory tools (_memory_search, _memory_store, _memory_compress)
     * @node agent_register_memory @receiver agent_in @alias agentRegisterMemory
     * @param agentIn — Agent object to register memory on (receiver: `this` in `x.registerMemory(...)`)
     * @param memoryConfig — MemoryConfig from Create Memory Config node (bundles database + embedding model + tuning parameters)
     * @returns agentOut — Agent with memory tools registered
     */
    function registerMemory(this: Agent, { agentIn: Struct, memoryConfig: Struct }): Struct;

    /**
     * Adds a connected app's MCP event as agent tools. Uses a short-lived app-to-app token (valid ~15 minutes) that is refreshed on every run.
     * @node agent_register_remote_mcp_tools @receiver agent_in @alias agentRegisterRemoteMcpTools
     * @param agentIn — Agent object to add the remote MCP tools to (receiver: `this` in `x.registerRemoteMcpTools(...)`)
     * @param flowRemoteAppId (optional) — Connected project that hosts the MCP event
     * @param flowRemoteEvent (optional) — MCP event of the selected project
     * @param flowRemoteEventMeta (optional) — Auto-filled by the editor when an event is selected
     * @param toolFilter — Optional list of tool names to include. Empty = all tools.
     * @param headers — Static registration authentication headers (for example Authorization or x-api-key). HMAC auth is not supported because each MCP request requires a fresh signature.
     * @returns agentOut — Agent object with the remote MCP tools registered
     */
    function registerRemoteMcpTools(this: Agent, { agentIn: Struct, flowRemoteAppId?: string, flowRemoteEvent?: string, flowRemoteEventMeta?: string, toolFilter: string[], headers: Struct }): Struct;

    /**
     * Enables Rig's built-in Thinking tool for reasoning capabilities
     * @node agent_register_thinking @receiver agent_in @alias agentRegisterThinking
     * @param agentIn — Agent object to enable thinking on (receiver: `this` in `x.registerThinking(...)`)
     * @returns agentOut — Agent object with thinking tool enabled
     */
    function registerThinking(this: Agent, { agentIn: Struct }): Struct;

    /**
     * Sets the system prompt for an Agent to guide its behavior
     * @node agent_set_system_prompt @receiver agent_in @alias agentSetSystemPrompt
     * @param agentIn — Agent object to enable thinking on (receiver: `this` in `x.setSystemPrompt(...)`)
     * @param systemPrompt (optional) — System prompt string to set for the agent
     * @returns agentOut — Agent object with thinking tool enabled
     */
    function setSystemPrompt(this: Agent, { agentIn: Struct, systemPrompt?: string }): Struct;
}

declare namespace ai {
    // === AI/Generative ===

    /**
     * Adds custom HTTP headers to a model for use with custom API endpoints
     * @node ai_generative_add_headers @alias aiGenerativeAddHeaders
     * @param model — Model to add headers to
     * @param header (optional) — HTTP header to add (name-value pair)
     * @returns modelOut — Model with custom headers applied
     * @impure has side effects / drives control flow
     */
    function addHeaders({ model: Struct, header?: Struct }): Struct;

    /**
     * Routes execution based on an LLM-evaluated yes/no decision
     * @node llm_branch @alias llmBranch
     * @param model — Bit representing the LLM to query
     * @param prompt — Statement/question that should result in a yes/no decision
     * @impure has side effects / drives control flow
     */
    function branch({ model: Struct, prompt: string }): void;

    /**
     * Uses an LLM plus a JSON schema to extract structured data from free-form text
     * @node llm_extractor @alias llmExtractor
     * @param model — Bit pointing to the LLM that will perform the extraction
     * @param schema — JSON Schema (or example JSON) describing the structure to extract
     * @param text — Raw text that should be structured via the schema
     * @param hint (optional) — Optional hint to guide the extraction (e.g. 'only extract individual line items, not totals')
     * @returns response — Structured JSON value that matches the schema
     * @returns stats — Token usage, cost, and model statistics
     * @impure has side effects / drives control flow
     */
    function extract({ model: Struct, schema: string, text: string, hint?: string }): { response: any, stats: Struct };

    /**
     * Extracts structured data by replaying an entire chat history through an LLM
     * @node llm_extractor_history @alias llmExtractorHistory
     * @param model — Bit pointing to the LLM that will perform the extraction
     * @param schema — JSON Schema (or example JSON) describing the structure to extract
     * @param history — Chat history to replay when extracting data
     * @param hint (optional) — Optional hint to guide the extraction (e.g. 'only extract individual line items, not totals')
     * @returns response — Structured JSON value that matches the schema
     * @returns stats — Token usage, cost, and model statistics
     * @impure has side effects / drives control flow
     */
    function extractFromHistory({ model: Struct, schema: string, history: Struct, hint?: string }): { response: any, stats: Struct };

    /**
     * Finds the best model based on certain selection criteria
     * @node ai_generative_find_model @alias aiGenerativeFindModel
     * @param preferences (optional) — Weights and requirements that guide model selection
     * @returns model — Bit describing the best-match model
     * @impure has side effects / drives control flow
     */
    function findModel({ preferences?: Struct }): Struct;

    /**
     * Invokes the configured model with the provided chat history. Set history streaming off to preserve and replay structured media responses.
     * @node ai_generative_invoke @alias aiGenerativeInvoke
     * @param model — Model
     * @param history — Chat History
     * @returns chunk
     * @returns result — Resulting Model Output
     * @returns stats — Token usage, cost, and model statistics
     * @impure has side effects / drives control flow
     */
    function invoke({ model: Struct, history: Struct }): { chunk: Struct, result: Struct, stats: Struct };

    /**
     * Invokes an LLM with a system prompt and user prompt, returning text and the full structured response.
     * @node ai_generative_invoke_simple @alias aiGenerativeInvokeSimple
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
    function invokeSimple({ model: Struct, systemPrompt?: string, prompt?: string, stream?: bool }): { token: string, chunk: Struct, result: string, response: Struct, stats: Struct };

    /**
     * Invokes an LLM that can call Flow tools/functions and routes each call to execution pins.
     * @node invoke_llm_with_tools @alias invokeLlmWithTools
     * @param model — Bit describing the provider/model to execute
     * @param history — Conversation history the model should continue from
     * @param tools (optional) — JSON array of tool/function definitions (OpenAI format)
     * @param toolChoice (optional) — Controls whether the model must, may, or must not call tools
     * @returns response — LLM response if the model answered directly without tool calls
     * @returns toolCallArgs — Parsed JSON arguments for the latest tool call
     * @returns stats — Token usage, cost, and model statistics
     * @impure has side effects / drives control flow
     */
    function invokeWithTools({ model: Struct, history: Struct, tools?: string, toolChoice?: string }): { response: Struct, toolCallArgs: Struct, stats: Struct };

    /**
     * Summarizes long text using an LLM with configurable strategies. Supports Map-Reduce (parallel, fast), Refine (sequential, coherent), Hierarchical (structure-aware), Hybrid (parallel + coherent), and Sliding Window (memory-efficient). Optional Chain of Density post-processing for optimal information density.
     * @node ai_llm_summarize @alias aiLlmSummarize
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
    function summarize({ model: Struct, text?: string, strategy?: string, densification?: string, instructions?: string, priorSummary?: string, chunkSize?: int, chunkOverlap?: int, trackEntities?: bool, concurrency?: int, maxIterations?: int, densitySteps?: int }): { summary: string, entities: string[], llmCalls: int };

    namespace embedding {
        // === AI/Embedding ===

        /**
         * Creates an embedding vector for a document string using a cached embedding model
         * @node embed_document @alias embedDocument
         * @param queryString — Document text that should be embedded
         * @param model — Cached embedding Bit containing the provider
         * @returns vector — Embedding vector returned by the model
         * @impure has side effects / drives control flow
         */
        function embedDocument({ queryString: string, model: Struct }): float[];

        /**
         * Embeds an image using a loaded model
         * @node embed_image @alias embedImage
         * @param image — The image to embed
         * @param model — The embedding model
         * @returns vector — The embedding vector
         * @impure has side effects / drives control flow
         */
        function embedImage({ image: Struct, model: Struct }): float[];

        /**
         * Embeds a query string using a loaded model
         * @node embed_query @alias embedQuery
         * @param queryString — The string to embed
         * @param model — The embedding model
         * @returns vector — The embedding vector
         * @impure has side effects / drives control flow
         */
        function embedQuery({ queryString: string, model: Struct }): float[];

        /**
         * Loads a model from a Bit
         * @node load_model @alias loadModel
         * @param bit — The Bit that contains the Model
         * @returns model — Model Out
         * @impure has side effects / drives control flow
         */
        function loadModel({ bit: Struct }): Struct;
    }

    namespace memory {
        // === AI/Memory ===

        /**
         * Assembles retrieved memory records into a token-budgeted context string for injection into agent system prompts
         * @node memory_build_context @receiver memory_config @alias memoryBuildContext
         * @param memoryConfig — MemoryConfig for token budget (receiver: `this` in `x.buildContext(...)`)
         * @param memories — Array of memory records from Search Memory node
         * @param header (optional) — Optional header text prepended to the context block
         * @returns contextText — Assembled memory context string, ready for system prompt injection
         * @returns tokenEstimate — Approximate token count of the assembled context
         * @impure has side effects / drives control flow
         */
        function buildContext(this: MemoryConfig, { memoryConfig: Struct, memories: Struct[], header?: string }): { contextText: string, tokenEstimate: int };

        /**
         * Compresses old memory observations into a summary using an LLM, then replaces them in the store. Runs the embedding model to store the summary vector.
         * @node memory_compress @receiver memory_config @alias memoryCompress
         * @param memoryConfig — MemoryConfig from Create Memory Config node (receiver: `this` in `x.compress(...)`)
         * @param observations — Array of memory records to compress (typically older observations from Search Memory)
         * @param model — LLM model Bit for generating the summary
         * @returns summaryText — The compressed summary text
         * @returns compressedCount — Number of observations that were compressed
         * @returns stats — Token usage and model statistics from the compaction LLM call
         * @impure has side effects / drives control flow
         */
        function compress(this: MemoryConfig, { memoryConfig: Struct, observations: Struct[], model: Struct }): { summaryText: string, compressedCount: int, stats: Struct };

        /**
         * Creates a MemoryConfig that bundles database, embedding model, and tuning parameters for all memory nodes
         * @node memory_create_config @alias memoryCreateConfig
         * @param database — LanceDB connection (from Open Database node). The table IS the scope boundary — use one table per user/session.
         * @param embeddingModel — Cached embedding model for vector search (from Load Embedding Model node)
         * @param maxContextTokens (optional) — Token budget for assembled memory context
         * @param recallStrategy (optional) — How to retrieve memories: recent_first (last N), relevance (vector similarity), hybrid (both)
         * @param recallTopK (optional) — Max items returned from vector search
         * @param autoCompress (optional) — Automatically compress old observations when threshold is reached
         * @param compressThreshold (optional) — Number of observations before triggering compression
         * @returns memoryConfig — Configured MemoryConfig — pass to any memory node
         */
        function createConfig({ database: Struct, embeddingModel: Struct, maxContextTokens?: int, recallStrategy?: string, recallTopK?: int, autoCompress?: bool, compressThreshold?: int }): Struct;

        /**
         * Runs LanceDB maintenance on the memory table: flush buffered writes, compact fragments, and update indices. Optional cleanup prunes versions older than seven days after maintenance.
         * @node memory_optimize @receiver memory_config @alias memoryOptimize
         * @param memoryConfig — MemoryConfig from Create Memory Config node (receiver: `this` in `x.optimize(...)`)
         * @param keepVersions (optional) — Retain all versions. Disable only to prune versions older than seven days after maintenance.
         * @impure has side effects / drives control flow
         */
        function optimize(this: MemoryConfig, { memoryConfig: Struct, keepVersions?: bool }): void;

        /**
         * Searches the memory store using the configured recall strategy (recent, relevance, or hybrid)
         * @node memory_search @receiver memory_config @alias memorySearch
         * @param memoryConfig — MemoryConfig from Create Memory Config node (receiver: `this` in `x.search(...)`)
         * @param query — Search query text — used for vector similarity and/or full-text search
         * @param roleFilter (optional) — Optional role filter (one of: user, assistant, observation, summary, context)
         * @returns results — Array of matching memory records (sorted by relevance/recency)
         * @returns resultCount — Number of results returned
         * @impure has side effects / drives control flow
         */
        function search(this: MemoryConfig, { memoryConfig: Struct, query: string, roleFilter?: string }): { results: Struct[], resultCount: int };

        /**
         * Embeds text and stores it as a memory observation in the configured LanceDB table
         * @node memory_store @receiver memory_config @alias memoryStore
         * @param memoryConfig — MemoryConfig from Create Memory Config node (receiver: `this` in `x.store(...)`)
         * @param content — Text content to store as a memory observation
         * @param role (optional) — Role of the message author
         * @returns observationCount — Total number of observations in the memory table after this insert
         * @impure has side effects / drives control flow
         */
        function store(this: MemoryConfig, { memoryConfig: Struct, content: string, role?: string }): int;

        // === AI/Memory/Graph ===

        /**
         * Extracts entities (nodes) and relationships (edges) from text using an LLM, returning structured arrays ready for graph insertion
         * @node kg_extract @receiver graph @alias kgExtract
         * @param graph — Graph connection from Open Graph Overlay node (receiver: `this` in `x.kgExtract(...)`)
         * @param text — Input text to extract entities and relationships from
         * @param nodeLabels — Allowed node labels for extraction (from overlay definition)
         * @param edgeLabels — Allowed edge labels for extraction (from overlay definition)
         * @returns errorMessage — Error details
         * @returns extractedNodes — Array of extracted entity objects with label, id, and properties
         * @returns extractedEdges — Array of extracted relationship objects with label, source, target, and properties
         * @returns entityCount — Total number of entities extracted
         * @impure has side effects / drives control flow
         */
        function kgExtract(this: NodeGraphConnection, { graph: Struct, text: string, nodeLabels: string[], edgeLabels: string[] }): { errorMessage: string, extractedNodes: Struct[], extractedEdges: Struct[], entityCount: int };

        /**
         * Retrieves context from a knowledge graph: embeds the query, finds matching nodes, then expands N hops to build structured context
         * @node kg_retrieve @receiver graph @alias kgRetrieve
         * @param graph — Graph connection from Open Graph Overlay node (receiver: `this` in `x.kgRetrieve(...)`)
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
        function kgRetrieve(this: NodeGraphConnection, { graph: Struct, query: string, nodeLabel: string, depth?: int, topK?: int, limit?: int }): { errorMessage: string, context: Struct, summaryText: string, nodeCount: int };

        /**
         * Converts a subgraph (nodes + edges) into a natural-language summary for LLM consumption
         * @node kg_summarize @receiver graph @alias kgSummarize
         * @param graph — Graph connection reference (for label metadata) (receiver: `this` in `x.kgSummarize(...)`)
         * @param subgraph — Subgraph payload (output from KG Retrieve, Neighbors, or Subgraph nodes)
         * @param maxTokens (optional) — Approximate maximum token budget for the summary (controls verbosity)
         * @param includeProperties (optional) — Whether to include node/edge properties in the summary
         * @returns summary — Natural-language summary of the subgraph
         * @returns nodeCount — Number of nodes in the input subgraph
         * @returns edgeCount — Number of edges in the input subgraph
         * @impure has side effects / drives control flow
         */
        function kgSummarize(this: NodeGraphConnection, { graph: Struct, subgraph: Struct, maxTokens?: int, includeProperties?: bool }): { summary: string, nodeCount: int, edgeCount: int };
    }

    namespace preferences {
        // === AI/Generative/Preferences ===

        /**
         * Creates a BitModelPreference struct used to guide model selection
         * @node ai_generative_make_preferences @alias aiGenerativeMakePreferences
         * @param multimodal (optional) — True if the target model must handle images
         * @returns preferences — Constructed BitModelPreference struct
         */
        function make({ multimodal?: bool }): Struct;

        /**
         * Adds a soft preference hint for downstream model selection
         * @node ai_generative_set_model_hint @receiver preferences_in @alias aiGenerativeSetModelHint
         * @param preferencesIn — Current model preference state (receiver: `this` in `x.setModelHint(...)`)
         * @param modelHint — Friendly hint describing the desired model family
         * @returns preferencesOut — Preferences with the new hint
         * @impure has side effects / drives control flow
         */
        function setModelHint(this: BitModelPreference, { preferencesIn: Struct, modelHint: string }): Struct;

        /**
         * Adjusts the relative weight for a specific capability preference
         * @node ai_generative_set_preference_weight @receiver preferences_in @alias aiGenerativeSetPreferenceWeight
         * @param preferencesIn — Current preference struct (receiver: `this` in `x.setWeight(...)`)
         * @param preferencesKey (optional) — Which capability weight to change
         * @param weight — Weight to set
         * @returns preferencesOut — Preferences carrying the new weight
         * @impure has side effects / drives control flow
         */
        function setWeight(this: BitModelPreference, { preferencesIn: Struct, preferencesKey?: string, weight: float }): Struct;
    }

    namespace processing {
        // === AI/Preprocessing ===

        /**
         * Splits long text into sized/overlapping chunks using the cached embedding model's splitter
         * @node chunk_text @alias chunkText
         * @param text — Source string that needs chunking
         * @param model — Cached embedding Bit providing the tokenizer/splitter
         * @param capacity (optional) — Max characters/tokens in each chunk
         * @param overlap (optional) — How many characters/tokens overlap between consecutive chunks
         * @param markdown (optional) — Use a Markdown-aware splitter (true) or the plain splitter
         * @returns chunks — Array of chunked text segments
         * @impure has side effects / drives control flow
         */
        function chunkText({ text: string, model: Struct, capacity?: int, overlap?: int, markdown?: bool }): string[];

        /**
         * Splits raw text locally using simple character-based chunking
         * @node chunk_text_char @alias chunkTextChar
         * @param text — Source string that should be chunked
         * @param capacity (optional) — Maximum characters per chunk
         * @param overlap (optional) — Character overlap between adjacent chunks
         * @param markdown (optional) — Use Markdown-aware splitting (true) or basic splitter
         * @returns chunks — Character chunk array
         * @impure has side effects / drives control flow
         */
        function chunkTextChar({ text: string, capacity?: int, overlap?: int, markdown?: bool }): string[];
    }

    namespace provider {
        // === AI/Generative/Provider ===

        /**
         * Prepares a Bit for Anthropic's Claude API using the provided credentials
         * @node ai_generative_build_anthropic @alias aiGenerativeBuildAnthropic
         * @param endpoint (optional) — Anthropic API endpoint
         * @param apiKey (optional) — Anthropic API key
         * @param modelId (optional) — Claude model identifier
         * @returns model — Bit containing the provider configuration
         * @impure has side effects / drives control flow
         */
        function anthropic({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Builds a model served by Atlas Cloud, a full-modal AI inference platform exposing a single OpenAI-compatible API (DeepSeek, Qwen, GLM, Kimi, MiniMax and more)
         * @node ai_generative_build_atlascloud @alias aiGenerativeBuildAtlascloud
         * @param endpoint (optional) — Atlas Cloud OpenAI-compatible base URL (override only for a proxy)
         * @param apiKey (optional) — Atlas Cloud API key used for authentication
         * @param modelId (optional) — Atlas Cloud model identifier to request (e.g., deepseek-ai/deepseek-v4-pro)
         * @returns model — Structured Bit describing the Atlas Cloud provider
         * @impure has side effects / drives control flow
         */
        function atlascloud({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Prepares a Bit for AWS Bedrock model endpoints
         * @node ai_generative_build_bedrock @alias aiGenerativeBuildBedrock
         * @param region (optional) — AWS Bedrock runtime region
         * @param endpoint (optional) — Optional Bedrock Runtime endpoint override. Leave empty to derive from region.
         * @param apiKey (optional) — Credential used for Bedrock runtime requests
         * @param modelId (optional) — AWS Bedrock model identifier
         * @returns model — Structured Bit describing the AWS Bedrock provider
         * @impure has side effects / drives control flow
         */
        function bedrock({ region?: string, endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Prepares a Bit for Cohere's API using the supplied credentials
         * @node ai_generative_build_cohere @alias aiGenerativeBuildCohere
         * @param endpoint (optional) — Cohere API endpoint (override for private deployments)
         * @param apiKey (optional) — Cohere API key
         * @param modelId (optional) — Cohere model identifier
         * @returns model — Bit containing the provider configuration
         * @impure has side effects / drives control flow
         */
        function cohere({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Prepares a Bit for Deepseek's API using the provided credentials
         * @node ai_generative_build_deepseek @alias aiGenerativeBuildDeepseek
         * @param endpoint (optional) — Deepseek API endpoint
         * @param apiKey (optional) — Deepseek API key
         * @param modelId (optional) — Deepseek model identifier
         * @returns model — Bit containing the provider configuration
         * @impure has side effects / drives control flow
         */
        function deepseek({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Prepares a Bit for Galadriel's verified endpoint using the provided credentials
         * @node ai_generative_build_galadriel @alias aiGenerativeBuildGaladriel
         * @param endpoint (optional) — Galadriel API endpoint
         * @param apiKey (optional) — Galadriel API key
         * @param modelId (optional) — Galadriel model identifier
         * @returns model — Bit containing the provider configuration
         * @impure has side effects / drives control flow
         */
        function galadriel({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Prepares a Bit for Google Gemini endpoints using the provided credentials
         * @node ai_generative_build_gemini @alias aiGenerativeBuildGemini
         * @param endpoint (optional) — Gemini REST endpoint
         * @param apiKey (optional) — Gemini API key
         * @param modelId (optional) — Gemini model identifier
         * @returns model — Bit containing the provider configuration
         * @impure has side effects / drives control flow
         */
        function gemini({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Prepares a Bit for Groq's API using the supplied endpoint and key
         * @node ai_generative_build_groq @alias aiGenerativeBuildGroq
         * @param endpoint (optional) — Groq-compatible API endpoint
         * @param apiKey (optional) — Groq API key
         * @param modelId (optional) — Groq-served model identifier
         * @returns model — Bit containing the provider configuration
         * @impure has side effects / drives control flow
         */
        function groq({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Builds the Huggingface model based on certain selection criteria
         * @node ai_generative_build_huggingface @alias aiGenerativeBuildHuggingface
         * @param endpoint (optional) — Router or custom inference endpoint to use for requests
         * @param apiKey (optional) — Token used for authenticating against the Hugging Face endpoint
         * @param modelId (optional) — Repository/model identifier to load (e.g. meta-llama/Meta-Llama-3-8B-Instruct)
         * @returns model — Structured Bit describing the Hugging Face provider
         * @impure has side effects / drives control flow
         */
        function huggingface({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Builds the Hyperbolic model based on certain selection criteria
         * @node ai_generative_build_hyperbolic @alias aiGenerativeBuildHyperbolic
         * @param endpoint (optional) — Public API endpoint or custom proxy to reach Hyperbolic
         * @param apiKey (optional) — Token used for authenticating against Hyperbolic
         * @param modelId (optional) — Repository slug or model identifier to load
         * @returns model — Structured Bit describing the Hyperbolic provider
         * @impure has side effects / drives control flow
         */
        function hyperbolic({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Connects to a locally running LM Studio server via its OpenAI-compatible API
         * @node ai_generative_build_lmstudio @alias aiGenerativeBuildLmstudio
         * @param endpoint (optional) — LM Studio server URL (default: http://localhost:1234)
         * @param modelId (optional) — Model identifier as shown in LM Studio (e.g. lmstudio-community/gemma-3-12b)
         * @returns model — Structured Bit describing the LM Studio provider
         * @impure has side effects / drives control flow
         */
        function lmstudio({ endpoint?: string, modelId?: string }): Struct;

        /**
         * Prepares a Bit for the MiniMax API using the provided credentials
         * @node ai_generative_build_minimax @alias aiGenerativeBuildMinimax
         * @param region (optional) — MiniMax API region used when no custom endpoint is provided
         * @param endpoint (optional) — Optional MiniMax API base URL override for a proxy
         * @param apiKey (optional) — MiniMax API key used for authentication
         * @param modelId (optional) — MiniMax model identifier to request
         * @returns model — Bit containing the provider configuration
         * @impure has side effects / drives control flow
         */
        function minimax({ region?: string, endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Builds the Mira model based on certain selection criteria
         * @node ai_generative_build_mira @alias aiGenerativeBuildMira
         * @param endpoint (optional) — Public Mira API endpoint or private gateway override
         * @param apiKey (optional) — Token used for authenticating against Mira
         * @param modelId (optional) — Model identifier or preset slug to deploy
         * @returns model — Structured Bit describing the Mira provider
         * @impure has side effects / drives control flow
         */
        function mira({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Builds the Mistral model based on certain selection criteria
         * @node ai_generative_build_mistral @alias aiGenerativeBuildMistral
         * @param endpoint (optional) — Public Mistral endpoint or private deployment URL
         * @param apiKey (optional) — Token used for authenticating against Mistral
         * @param modelId (optional) — Model identifier or preset slug to load
         * @returns model — Structured Bit describing the Mistral provider
         * @impure has side effects / drives control flow
         */
        function mistral({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Builds the Moonshot AI model based on certain selection criteria
         * @node ai_generative_build_moonshot @alias aiGenerativeBuildMoonshot
         * @param endpoint (optional) — Public Moonshot endpoint or custom proxy URL
         * @param apiKey (optional) — Token used for authenticating against Moonshot
         * @param modelId (optional) — Model identifier or preset slug (e.g., moonshot-v1-8k)
         * @returns model — Structured Bit describing the Moonshot provider
         * @impure has side effects / drives control flow
         */
        function moonshot({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Builds a model via the Mozilla any-llm gateway (OpenAI-compatible). Supports both self-hosted gateways and the managed platform at any-llm.ai
         * @node ai_generative_build_mozilla @alias aiGenerativeBuildMozilla
         * @param endpoint (optional) — Mozilla any-llm gateway base URL (e.g. http://localhost:8000/v1 for self-hosted or https://api.any-llm.ai/v1 for managed platform)
         * @param apiKey (optional) — API key for authenticating against the any-llm gateway or platform
         * @param modelId (optional) — Model identifier in provider:model format (e.g. openai:gpt-4o, anthropic:claude-sonnet-4-20250514)
         * @returns model — Structured Bit describing the Mozilla any-llm provider
         * @impure has side effects / drives control flow
         */
        function mozilla({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Builds the Ollama model based on certain selection criteria
         * @node ai_generative_build_ollama @alias aiGenerativeBuildOllama
         * @param endpoint (optional) — Local or remote Ollama HTTP endpoint
         * @param modelId (optional) — Model identifier/tag to run (must exist on the Ollama host)
         * @returns model — Structured Bit describing the Ollama provider
         * @impure has side effects / drives control flow
         */
        function ollama({ endpoint?: string, modelId?: string }): Struct;

        /**
         * Prepares a Bit for OpenAI or Azure OpenAI endpoints with the provided credentials
         * @node ai_generative_build_openai @alias aiGenerativeBuildOpenai
         * @param provider (optional) — Choose OpenAI cloud or Azure OpenAI
         * @param endpoint (optional) — Base API endpoint (override for Azure or proxies)
         * @param apiKey (optional) — API key or Azure key used for authentication
         * @returns model — Bit containing the provider configuration
         * @impure has side effects / drives control flow
         */
        function openai({ provider?: string, endpoint?: string, apiKey?: string }): Struct;

        /**
         * Builds the OpenRouter model based on certain selection criteria
         * @node ai_generative_build_openrouter @alias aiGenerativeBuildOpenrouter
         * @param endpoint (optional) — OpenRouter base URL or regional proxy
         * @param apiKey (optional) — Token used for authenticating against OpenRouter
         * @param modelId (optional) — Model identifier from OpenRouter's catalog
         * @returns model — Structured Bit describing the OpenRouter provider
         * @impure has side effects / drives control flow
         */
        function openrouter({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Builds the Perplexity model based on certain selection criteria
         * @node ai_generative_build_perplexity @alias aiGenerativeBuildPerplexity
         * @param endpoint (optional) — Perplexity API endpoint or self-hosted base URL
         * @param apiKey (optional) — Token used for authenticating against Perplexity
         * @param modelId (optional) — Model identifier or preset slug to request
         * @returns model — Structured Bit describing the Perplexity provider
         * @impure has side effects / drives control flow
         */
        function perplexity({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Builds the Together AI model based on certain selection criteria
         * @node ai_generative_build_together @alias aiGenerativeBuildTogether
         * @param endpoint (optional) — Together API endpoint or regional proxy
         * @param apiKey (optional) — Token used for authenticating against Together
         * @param modelId (optional) — Model identifier or preset slug to request
         * @returns model — Structured Bit describing the Together provider
         * @impure has side effects / drives control flow
         */
        function together({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Prepares a Bit for Google Vertex AI Gemini endpoints using ADC or service account credentials
         * @node ai_generative_build_vertex @alias aiGenerativeBuildVertex
         * @param projectId (optional) — Google Cloud project ID. Leave empty to use GOOGLE_CLOUD_PROJECT or the service account project_id.
         * @param location (optional) — Vertex AI location
         * @param serviceAccountJson (optional) — Optional Google Cloud service account key JSON. Leave empty to use Application Default Credentials.
         * @param accessToken (optional) — Optional OAuth access token. Prefer ADC or a service account for long-running flows.
         * @param modelId (optional) — Vertex AI Gemini model identifier
         * @returns model — Bit containing the provider configuration
         * @impure has side effects / drives control flow
         */
        function vertex({ projectId?: string, location?: string, serviceAccountJson?: string, accessToken?: string, modelId?: string }): Struct;

        /**
         * Builds the VoyageAI model based on certain selection criteria
         * @node ai_generative_build_voyageai @alias aiGenerativeBuildVoyageai
         * @param endpoint (optional) — VoyageAI API base URL or custom proxy
         * @param apiKey (optional) — Token used for authenticating against VoyageAI
         * @param modelId (optional) — Model identifier or preset slug to use
         * @returns model — Structured Bit describing the VoyageAI provider
         * @impure has side effects / drives control flow
         */
        function voyageai({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;

        /**
         * Builds the xAI model based on certain selection criteria
         * @node ai_generative_build_xai @alias aiGenerativeBuildXai
         * @param endpoint (optional) — xAI API endpoint or custom proxy
         * @param apiKey (optional) — Token used for authenticating against xAI
         * @param modelId (optional) — Model identifier or preset slug to request (e.g., grok-2-1212)
         * @returns model — Structured Bit describing the xAI provider
         * @impure has side effects / drives control flow
         */
        function xai({ endpoint?: string, apiKey?: string, modelId?: string }): Struct;
    }

    namespace response {
        // === AI/Generative/Response ===

        /**
         * Wraps an arbitrary string in a synthetic streaming chunk
         * @node ai_generative_llm_chunk_from_string @alias aiGenerativeLlmChunkFromString
         * @param content (optional) — Plain text that should stream to clients
         * @returns chunk — Response chunk built from the provided text
         */
        function chunkFromString({ content?: string }): Struct;

        /**
         * Wraps a plain string into a synthetic LLM response object for downstream tooling.
         * @node ai_generative_llm_response_from_string @alias aiGenerativeLlmResponseFromString
         * @param content (optional) — Plain assistant text that should be wrapped into a Response object.
         * @returns response — LLM-style Response struct containing the provided content as a single assistant message.
         */
        function fromString({ content?: string }): Struct;

        /**
         * Extracts the content string from the last assistant message in a response
         * @node ai_generative_llm_response_last_content @receiver response @alias aiGenerativeLlmResponseLastContent
         * @param response — LLM response to extract from (receiver: `this` in `x.lastContent(...)`)
         * @returns content — Content string from the last message
         * @returns success — Whether content was successfully extracted
         * @returns parts — Ordered text and media content parts
         * @returns images — Image URLs or data URIs
         * @returns audio — Audio URLs or data URIs
         * @returns videos — Video URLs or data URIs
         * @returns documents — Document URLs or data URIs
         * @returns reasoning — Displayable reasoning returned by the model
         */
        function lastContent(this: Response, { response: Struct }): { content: string, success: bool, parts: Struct[], images: string[], audio: string[], videos: string[], documents: string[], reasoning: string };

        /**
         * Extracts the last assistant message from a response
         * @node ai_generative_llm_response_last_message @receiver response @alias aiGenerativeLlmResponseLastMessage
         * @param response — LLM response to inspect (receiver: `this` in `x.lastMessage(...)`)
         * @returns message — Last message from the response
         * @returns success — Whether a message was successfully extracted
         */
        function lastMessage(this: Response, { response: Struct }): { message: Struct, success: bool };

        /**
         * Creates an empty Response struct for manual composition
         * @node ai_generative_llm_response_make @alias aiGenerativeLlmResponseMake
         * @returns response — Empty Response ready to populate
         */
        function make(): Struct;

        /**
         * Appends a streaming chunk onto a response
         * @node ai_generative_llm_response_push_chunk @receiver response @alias aiGenerativeLlmResponsePushChunk
         * @param response — Response object that should receive the chunk (receiver: `this` in `x.pushChunk(...)`)
         * @param chunk — Chunk to append
         * @returns responseOut — Response including the appended chunk
         * @impure has side effects / drives control flow
         */
        function pushChunk(this: Response, { response: Struct, chunk: Struct }): Struct;

        // === AI/Generative/Response/Chunk ===

        /**
         * Extracts the latest streamed token from a response chunk
         * @node ai_generative_llm_response_chunk_get_token @receiver chunk @alias aiGenerativeLlmResponseChunkGetToken
         * @param chunk — Response chunk that carries streamed tokens (receiver: `this` in `x.getToken(...)`)
         * @returns token — Most recent streamed token
         */
        function getToken(this: ResponseChunk, { chunk: Struct }): string;

        // === AI/Generative/Response/Message ===

        /**
         * Extracts the text content field from a response message
         * @node ai_generative_llm_response_message_get_content @receiver message @alias aiGenerativeLlmResponseMessageGetContent
         * @param message — Message to extract content from (receiver: `this` in `x.getContent(...)`)
         * @returns content — Content string from the message
         * @returns success — Whether content was successfully extracted
         * @returns parts — Ordered text and media content parts
         * @returns images — Image URLs or data URIs
         * @returns audio — Audio URLs or data URIs
         * @returns videos — Video URLs or data URIs
         * @returns documents — Document URLs or data URIs
         * @returns reasoning — Displayable reasoning returned by the model
         */
        function getContent(this: ResponseMessage, { message: Struct }): { content: string, success: bool, parts: Struct[], images: string[], audio: string[], videos: string[], documents: string[], reasoning: string };

        /**
         * Extracts the author role string from a response message
         * @node ai_generative_llm_response_message_get_role @receiver message @alias aiGenerativeLlmResponseMessageGetRole
         * @param message — Message to extract the role from (receiver: `this` in `x.getRole(...)`)
         * @returns role — Role string from the message
         */
        function getRole(this: ResponseMessage, { message: Struct }): string;
    }
}

declare namespace chat {
    // === Events/Chat ===

    /**
     * Pulls down image, audio, video, and document attachments referenced in the latest chat message
     * @node ai_gen_llm_history_extract_attachments @alias aiGenLlmHistoryExtractAttachments
     * @param history — Chat history whose final message may contain media parts
     * @param attachments (optional) — Existing attachments to merge with new downloads
     * @returns paths — Virtual file paths pointing to cached attachments
     * @impure has side effects / drives control flow
     */
    function extractAttachments({ history: Struct, attachments?: Struct[] }): Struct[];
}

declare namespace github {
    namespace copilot {
        // === AI/GitHub/Copilot/Chat ===

        /**
         * Aborts the current message processing
         * @node copilot_abort @receiver session @alias copilotAbort
         * @param session — Copilot session (receiver: `this` in `x.abort(...)`)
         * @impure has side effects / drives control flow
         */
        function abort(this: CopilotSessionHandle, { session: Struct }): void;

        /**
         * Sends a message to Copilot and waits for complete response. Supports history input for context.
         * @node copilot_send_and_wait @receiver session @alias copilotSendAndWait
         * @param session — Copilot session (receiver: `this` in `x.sendMessage(...)`)
         * @param prompt — Message to send
         * @param history — Optional chat history for context (same format as Model Invoke)
         * @returns response — Complete response text
         * @returns result — Response in standard model format (matches Model Invoke)
         * @returns stats — Token usage, cost, and model statistics
         * @impure has side effects / drives control flow
         */
        function sendMessage(this: CopilotSessionHandle, { session: Struct, prompt: string, history: Struct }): { response: string, result: Struct, stats: Struct };

        /**
         * Sends a message to Copilot and streams the response. Supports history input and matches Model Invoke interface.
         * @node copilot_send_streaming @receiver session @alias copilotSendStreaming
         * @param session — Copilot session (receiver: `this` in `x.streamMessage(...)`)
         * @param prompt — Message to send
         * @param history — Optional chat history for context (same format as Model Invoke)
         * @returns chunk — Current streaming chunk (matches Model Invoke ResponseChunk format)
         * @returns result — Complete response (matches Model Invoke Response format)
         * @returns fullResponse — Complete accumulated response text
         * @returns stats — Token usage, cost, and model statistics
         * @impure has side effects / drives control flow
         */
        function streamMessage(this: CopilotSessionHandle, { session: Struct, prompt: string, history: Struct }): { chunk: Struct, result: Struct, fullResponse: string, stats: Struct };

        // === AI/GitHub/Copilot/Client ===

        /**
         * Builds a local Copilot client configuration (stdio-based). Requires 'copilot' CLI to be installed and in PATH, or specify the CLI path explicitly.
         * @node copilot_local_client_builder @alias copilotLocalClientBuilder
         * @param logLevel (optional) — Client log level
         * @param cliPath (optional) — Optional path to Copilot CLI executable. If not set, searches PATH and COPILOT_CLI_PATH env var.
         * @returns clientConfig — Local client configuration
         */
        function localClientConfig({ logLevel?: string, cliPath?: string }): Struct;

        /**
         * Builds a server/remote Copilot client configuration (TCP-based)
         * @node copilot_server_client_builder @alias copilotServerClientBuilder
         * @param url — TCP endpoint URL (e.g., tcp://localhost:3000)
         * @param logLevel (optional) — Client log level
         * @returns clientConfig — Server client configuration
         */
        function serverClientConfig({ url: string, logLevel?: string }): Struct;

        /**
         * Starts a local Copilot client using stdio. Requires 'copilot' CLI installed.
         * @node copilot_local_client_start @alias copilotLocalClientStart
         * @param clientConfig — Local client configuration
         * @returns client — Running client handle
         * @returns errorMessage — Error message if startup fails
         * @impure has side effects / drives control flow
         */
        function startLocalClient({ clientConfig: Struct }): { client: Struct, errorMessage: string };

        /**
         * Starts a server/remote Copilot client using TCP
         * @node copilot_server_client_start @alias copilotServerClientStart
         * @param clientConfig — Server client configuration
         * @returns client — Running client handle
         * @returns errorMessage — Error message if connection fails
         * @impure has side effects / drives control flow
         */
        function startServerClient({ clientConfig: Struct }): { client: Struct, errorMessage: string };

        /**
         * Gracefully stops a running Copilot client (local or server)
         * @node copilot_client_stop @receiver client @alias copilotClientStop
         * @param client — Client handle to stop (receiver: `this` in `x.stop(...)`)
         * @impure has side effects / drives control flow
         */
        function stop(this: CopilotClientHandle, { client: Struct }): void;

        // === AI/GitHub/Copilot/Config ===

        /**
         * Configures a custom agent
         * @node copilot_custom_agent @alias copilotCustomAgent
         * @param name — Agent identifier
         * @param displayName (optional) — Human-readable agent name
         * @param description (optional) — Agent description
         * @param prompt — Agent system prompt
         * @returns agent — Custom agent configuration
         */
        function customAgent({ name: string, displayName?: string, description?: string, prompt: string }): Struct;

        /**
         * Configures infinite session with automatic context compaction
         * @node copilot_infinite_session @alias copilotInfiniteSession
         * @param enabled (optional) — Enable infinite sessions
         * @param backgroundThreshold (optional) — Background compaction threshold (0.0-1.0)
         * @param exhaustionThreshold (optional) — Buffer exhaustion threshold (0.0-1.0)
         * @returns config — Infinite session configuration
         */
        function infiniteSession({ enabled?: bool, backgroundThreshold?: float, exhaustionThreshold?: float }): Struct;

        /**
         * Configures a custom provider (Bring Your Own Key)
         * @node copilot_provider_config @alias copilotProviderConfig
         * @param baseUrl — Provider API base URL (e.g., https://api.openai.com/v1)
         * @param apiKey — API key for authentication
         * @param model (optional) — Model ID to use
         * @returns config — Provider configuration
         */
        function providerConfig({ baseUrl: string, apiKey: string, model?: string }): Struct;

        /**
         * Configures the system message for the session
         * @node copilot_system_message @alias copilotSystemMessage
         * @param content — System message content
         * @param mode (optional) — Replace or Append to default system message
         * @returns config — System message configuration
         */
        function systemMessage({ content: string, mode?: string }): Struct;

        // === AI/GitHub/Copilot/MCP ===

        /**
         * Configures an HTTP/SSE MCP server for remote tool integration
         * @node copilot_mcp_http_server @alias copilotMcpHttpServer
         * @param url — HTTP endpoint URL
         * @param tools (optional) — Tool filter (use ["*"] for all tools)
         * @param timeout (optional) — Server timeout in milliseconds
         * @returns config — MCP server configuration
         */
        function mcpHttpServer({ url: string, tools?: string[], timeout?: int }): Struct;

        /**
         * Configures a local/stdio MCP server for tool integration
         * @node copilot_mcp_local_server @alias copilotMcpLocalServer
         * @param command — Command to execute (e.g., npx, python)
         * @param args (optional) — Command arguments
         * @param tools (optional) — Tool filter (use ["*"] for all tools)
         * @param timeout (optional) — Server timeout in milliseconds
         * @returns config — MCP server configuration
         */
        function mcpLocalServer({ command: string, args?: string[], tools?: string[], timeout?: int }): Struct;

        // === AI/GitHub/Copilot/Session ===

        /**
         * Creates a new Copilot chat session
         * @node copilot_create_session @receiver client @alias copilotCreateSession
         * @param client — Running Copilot client (receiver: `this` in `x.createSession(...)`)
         * @param config — Session configuration (from Session Builder)
         * @returns session — Session handle
         * @impure has side effects / drives control flow
         */
        function createSession(this: CopilotClientHandle, { client: Struct, config: Struct }): Struct;

        /**
         * Destroys a Copilot session
         * @node copilot_destroy_session @receiver session @alias copilotDestroySession
         * @param session — Session handle to destroy (receiver: `this` in `x.destroySession(...)`)
         * @impure has side effects / drives control flow
         */
        function destroySession(this: CopilotSessionHandle, { session: Struct }): void;

        /**
         * Builds a complete Copilot session configuration with all options
         * @node copilot_session_builder @alias copilotSessionBuilder
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
        function sessionConfig({ model?: string, streaming?: bool, systemMessage?: string, systemMode?: string, infiniteEnabled?: bool, backgroundThreshold?: float, exhaustionThreshold?: float, provider: Struct, tools: Struct[], customAgents: Struct[], mcpServers: any }): Struct;

        // === AI/GitHub/Copilot/Tools ===

        /**
         * Configures an agent tool with parameters
         * @node copilot_tool_config @alias copilotToolConfig
         * @param name — Tool name
         * @param description — Tool description
         * @param schema (optional) — Tool parameters JSON schema
         * @returns tool — Configured tool
         */
        function toolConfig({ name: string, description: string, schema?: Struct }): Struct;

        /**
         * Combines multiple tools into a list for session configuration
         * @node copilot_tool_list @alias copilotToolList
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
        function toolList({ tool1: Struct, tool2: Struct, tool3: Struct, tool4: Struct, tool5: Struct, tool6: Struct, tool7: Struct, tool8: Struct }): Struct[];

        // === AI/GitHub/Copilot/Utilities ===

        /**
         * Gets the authentication status of the Copilot client
         * @node copilot_get_auth_status @receiver client @alias copilotGetAuthStatus
         * @param client — Copilot client handle (receiver: `this` in `x.getAuthStatus(...)`)
         * @returns isAuthenticated — Whether the user is authenticated
         * @returns login — GitHub username if authenticated
         */
        function getAuthStatus(this: CopilotClientHandle, { client: Struct }): { isAuthenticated: bool, login: string };

        /**
         * Lists available Copilot models
         * @node copilot_get_models @receiver client @alias copilotGetModels
         * @param client — Copilot client handle (receiver: `this` in `x.getModels(...)`)
         * @returns models — Array of available model names
         */
        function getModels(this: CopilotClientHandle, { client: Struct }): string[];

        /**
         * Gets the version of the Copilot CLI
         * @node copilot_get_version @receiver client @alias copilotGetVersion
         * @param client — Copilot client handle (receiver: `this` in `x.getVersion(...)`)
         * @returns version — CLI version string
         */
        function getVersion(this: CopilotClientHandle, { client: Struct }): string;

        /**
         * Checks if a Copilot client is connected and ready
         * @node copilot_client_status @receiver client @alias copilotClientStatus
         * @param client — Copilot client handle (receiver: `this` in `x.status(...)`)
         * @returns isConnected — Whether the client is connected
         * @returns clientId — Client identifier
         */
        function status(this: CopilotClientHandle, { client: Struct }): { isConnected: bool, clientId: string };
    }
}

declare namespace history {
    // === AI/Generative/History ===

    /**
     * Appends a chat message to the end of a history
     * @node ai_generative_add_history_message @receiver history @alias aiGenerativeAddHistoryMessage
     * @param history — Chat history to append to (receiver: `this` in `x.addMessage(...)`)
     * @param message — Message that should be appended
     * @returns historyOut — History including the new message
     * @impure has side effects / drives control flow
     */
    function addMessage(this: History, { history: Struct, message: Struct }): Struct;

    /**
     * Clears all messages from a ChatHistory
     * @node ai_generative_clear_history @receiver history @alias aiGenerativeClearHistory
     * @param history — ChatHistory (receiver: `this` in `x.clear(...)`)
     * @returns historyOut — Cleared ChatHistory
     * @impure has side effects / drives control flow
     */
    function clear(this: History, { history: Struct }): Struct;

    /**
     * Creates a Chat History from Messages
     * @node ai_generative_from_messages @alias aiGenerativeFromMessages
     * @param modelName (optional) — Model Name
     * @param messages — Chat Messages
     * @returns history — ChatHistory
     */
    function fromMessages({ modelName?: string, messages: Struct[] }): Struct;

    /**
     * Creates a ChatHistory Struct from String (as User Message)
     * @node ai_generative_history_from_string @alias aiGenerativeHistoryFromString
     * @param modelName (optional) — Model Name
     * @param message — User Message String
     * @returns history — ChatHistory
     */
    function fromString({ modelName?: string, message: string }): Struct;

    /**
     * Extracts the first system-level message from a chat history for downstream use
     * @node ai_generative_get_system_prompt @receiver history @alias aiGenerativeGetSystemPrompt
     * @param history — Chat history that contains the system prompt (receiver: `this` in `x.getSystemPrompt(...)`)
     * @returns systemPrompt — Extracted system-level message
     * @returns success — True when a system message was located
     */
    function getSystemPrompt(this: History, { history: Struct }): { systemPrompt: Struct, success: bool };

    /**
     * Creates a ChatHistory struct
     * @node ai_generative_make_history @alias aiGenerativeMakeHistory
     * @param modelName (optional) — Model Name
     * @returns history — ChatHistory
     */
    function make({ modelName?: string }): Struct;

    /**
     * Removes and returns the last message in a chat history
     * @node ai_generative_pop_history_message @receiver history @alias aiGenerativePopHistoryMessage
     * @param history — History to remove the message from (receiver: `this` in `x.popMessage(...)`)
     * @returns historyOut — History after removing the message
     * @returns message — Removed message
     * @impure has side effects / drives control flow
     */
    function popMessage(this: History, { history: Struct }): { historyOut: Struct, message: Struct };

    /**
     * Stores the frequency penalty parameter used by LLM sampling
     * @node ai_generative_set_history_frequency_penalty @receiver history @alias aiGenerativeSetHistoryFrequencyPenalty
     * @param history — Existing chat history to update (receiver: `this` in `x.setFrequencyPenalty(...)`)
     * @param frequencyPenalty — Penalty applied when token frequency increases
     * @returns historyOut — History updated with frequency penalty
     * @impure has side effects / drives control flow
     */
    function setFrequencyPenalty(this: History, { history: Struct, frequencyPenalty: float }): Struct;

    /**
     * Stores the maximum completion tokens allowed for future calls
     * @node ai_generative_set_history_max_tokens @receiver history @alias aiGenerativeSetHistoryMaxTokens
     * @param history — Existing chat history to update (receiver: `this` in `x.setMaxTokens(...)`)
     * @param maxTokens — Maximum number of completion tokens
     * @returns historyOut — History updated with the max tokens limit
     * @impure has side effects / drives control flow
     */
    function setMaxTokens(this: History, { history: Struct, maxTokens: int }): Struct;

    /**
     * Stores how many completions to request in downstream LLM calls
     * @node ai_generative_set_history_n @receiver history @alias aiGenerativeSetHistoryN
     * @param history — Existing chat history to update (receiver: `this` in `x.setN(...)`)
     * @param n — Number of completions (u32)
     * @returns historyOut — History including the completion count
     * @impure has side effects / drives control flow
     */
    function setN(this: History, { history: Struct, n: int }): Struct;

    /**
     * Stores the presence penalty parameter used for discouraging repetition
     * @node ai_generative_set_history_presence_penalty @receiver history @alias aiGenerativeSetHistoryPresencePenalty
     * @param history — Existing chat history to update (receiver: `this` in `x.setPresencePenalty(...)`)
     * @param presencePenalty — Penalty applied when a token already appeared
     * @returns historyOut — History updated with the presence penalty
     * @impure has side effects / drives control flow
     */
    function setPresencePenalty(this: History, { history: Struct, presencePenalty: float }): Struct;

    /**
     * Configures the structured response format expected from later LLM calls
     * @node ai_generative_set_history_response_format @receiver history @alias aiGenerativeSetHistoryResponseFormat
     * @param history — Existing chat history to update (receiver: `this` in `x.setResponseFormat(...)`)
     * @param responseFormat — JSON schema or `string` that shapes responses
     * @returns historyOut — History updated with the response format
     * @impure has side effects / drives control flow
     */
    function setResponseFormat(this: History, { history: Struct, responseFormat: any }): Struct;

    /**
     * Stores an optional randomness seed alongside the chat history
     * @node ai_generative_set_history_seed @receiver history @alias aiGenerativeSetHistorySeed
     * @param history — Existing chat history to update (receiver: `this` in `x.setSeed(...)`)
     * @param seed — Deterministic seed value (u32)
     * @returns historyOut — History including the new seed
     * @impure has side effects / drives control flow
     */
    function setSeed(this: History, { history: Struct, seed: int }): Struct;

    /**
     * Stores one or more stop sequences to truncate future completions
     * @node ai_generative_set_history_stop_words @receiver history @alias aiGenerativeSetHistoryStopWords
     * @param history — Existing chat history to update (receiver: `this` in `x.setStopWords(...)`)
     * @param stopWords — Strings that should stop generation
     * @returns historyOut — History updated with stop sequences
     * @impure has side effects / drives control flow
     */
    function setStopWords(this: History, { history: Struct, stopWords: string[] }): Struct;

    /**
     * Stores whether downstream LLM invocations should stream tokens
     * @node ai_generative_set_history_stream @receiver history @alias aiGenerativeSetHistoryStream
     * @param history — Existing chat history to update (receiver: `this` in `x.setStream(...)`)
     * @param stream (optional) — Whether streaming tokens should be requested
     * @returns historyOut — History updated with the stream setting
     * @impure has side effects / drives control flow
     */
    function setStream(this: History, { history: Struct, stream?: bool }): Struct;

    /**
     * Creates or replaces the system prompt within a chat history before invoking an LLM
     * @node ai_generative_set_system_prompt_message @receiver history @alias aiGenerativeSetSystemPromptMessage
     * @param history — Existing chat history to modify (receiver: `this` in `x.setSystemPrompt(...)`)
     * @param message (optional) — System-level prompt text
     * @returns historyOut — History including the new system prompt
     * @impure has side effects / drives control flow
     */
    function setSystemPrompt(this: History, { history: Struct, message?: string }): Struct;

    /**
     * Stores the sampling temperature used for later LLM invocations
     * @node ai_generative_set_history_temperature @receiver history @alias aiGenerativeSetHistoryTemperature
     * @param history — Existing chat history to update (receiver: `this` in `x.setTemperature(...)`)
     * @param temperature — Sampling temperature (0-2)
     * @returns historyOut — History including the temperature setting
     * @impure has side effects / drives control flow
     */
    function setTemperature(this: History, { history: Struct, temperature: float }): Struct;

    /**
     * Stores the thinking level that downstream model invocations should use
     * @node ai_generative_set_history_thinking @receiver history @alias aiGenerativeSetHistoryThinking
     * @param history — Existing chat history to update (receiver: `this` in `x.setThinking(...)`)
     * @param thinking (optional) — Reasoning effort for downstream models: off, low, mid, or high
     * @returns historyOut — History updated with the thinking mode
     * @impure has side effects / drives control flow
     */
    function setThinking(this: History, { history: Struct, thinking?: string }): Struct;

    /**
     * Stores the nucleus sampling (top-p) parameter alongside the chat history
     * @node ai_generative_set_history_top_p @receiver history @alias aiGenerativeSetHistoryTopP
     * @param history — Existing chat history to update (receiver: `this` in `x.setTopP(...)`)
     * @param topP — Nucleus sampling probability mass (0-1)
     * @returns historyOut — History including the top-p value
     * @impure has side effects / drives control flow
     */
    function setTopP(this: History, { history: Struct, topP: float }): Struct;

    /**
     * Updates the user identifier stored alongside the chat history
     * @node ai_generative_set_history_user @receiver history @alias aiGenerativeSetHistoryUser
     * @param history — Existing chat history to update (receiver: `this` in `x.setUser(...)`)
     * @param user — User identifier or label to attach
     * @returns historyOut — History reflecting the new user metadata
     * @impure has side effects / drives control flow
     */
    function setUser(this: History, { history: Struct, user: string }): Struct;

    // === AI/Generative/History/Message ===

    /**
     * Extracts text content from a chat message, flattening multi-part payloads
     * @node ai_generative_message_extract_content @receiver message @alias aiGenerativeMessageExtractContent
     * @param message — Message whose text content will be extracted (receiver: `this` in `x.extractContent(...)`)
     * @returns content — Concatenated text content
     * @returns parts — Ordered text and media content parts
     * @returns images — Image URLs or data URIs
     * @returns audio — Audio URLs or data URIs
     * @returns videos — Video URLs or data URIs
     * @returns documents — Document URLs or data URIs
     */
    function extractContent(this: HistoryMessage, { message: Struct }): { content: string, parts: Struct[], images: string[], audio: string[], videos: string[], documents: string[] };

    /**
     * Creates a chat message with text, image, audio, video, or document content and optional tool metadata
     * @node ai_generative_make_history_message @alias aiGenerativeMakeHistoryMessage
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
    function makeMessage({ role?: string, type?: string, text?: string, image?: string, audio?: string, video?: string, document?: string, detail?: string, mime?: string, toolCallId?: string }): Struct;

    /**
     * Appends text, image, audio, video, or document parts onto a chat message
     * @node ai_generative_push_content @receiver message @alias aiGenerativePushContent
     * @param message — Message to extend (receiver: `this` in `x.pushContent(...)`)
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
    function pushContent(this: HistoryMessage, { message: Struct, type?: string, text?: string, image?: string, audio?: string, video?: string, document?: string, detail?: string, mime?: string }): Struct;
}
