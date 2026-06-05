// processing — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

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


// === Processing/Privacy ===

/**
 * Configure which PII types to detect. Connect to PII Mask nodes for fine-grained control.
 * @param email (optional) — Detect email addresses (e.g., user@example.com)
 * @param phone (optional) — Detect phone numbers (international formats)
 * @param url (optional) — Detect URLs and web addresses
 * @param ipAddress (optional) — Detect IPv4 and IPv6 addresses
 * @param creditCard (optional) — Detect credit card numbers (13-19 digits)
 * @param iban (optional) — Detect IBAN bank account numbers (international)
 * @param vatEu (optional) — Detect EU VAT numbers
 * @param ssn (optional) — Detect US Social Security Numbers (XXX-XX-XXXX)
 * @param germanTaxId (optional) — Detect German Steuer-ID (11 digits)
 * @param ahvSwiss (optional) — Detect Swiss AHV numbers (756.XXXX.XXXX.XX)
 * @param svnrAustria (optional) — Detect Austrian social insurance numbers
 * @param passport (optional) — Detect passport numbers (various formats)
 * @param driversLicense (optional) — Detect driver's license numbers (basic patterns)
 * @param addressUs (optional) — Detect US street addresses
 * @param addressDe (optional) — Detect German addresses (Straße, Platz, Weg, etc.)
 * @param postcodeUk (optional) — Detect UK postcodes
 * @param postcodeDe (optional) — Detect German postcodes (5 digits)
 * @param date (optional) — Detect date patterns (DD/MM/YYYY, YYYY-MM-DD, etc.)
 * @returns options — PII Detection Options configuration struct
 */
declare function processingPiiDetectionOptions({ email?: bool, phone?: bool, url?: bool, ipAddress?: bool, creditCard?: bool, iban?: bool, vatEu?: bool, ssn?: bool, germanTaxId?: bool, ahvSwiss?: bool, svnrAustria?: bool, passport?: bool, driversLicense?: bool, addressUs?: bool, addressDe?: bool, postcodeUk?: bool, postcodeDe?: bool, date?: bool }): Struct;

/**
 * Masks Personally Identifiable Information using regex patterns. Detects emails, phones, SSNs, credit cards, IBANs, addresses (US/DE/UK), and more. For names or contextual PII, use the AI-based node.
 * @param text (optional) — The text to scan for PII
 * @param options (optional) — Configuration for which PII types to detect. Connect a PII Detection Options node or use defaults (all enabled).
 * @param detectEmail (optional) — Override: Enable/disable email detection
 * @param detectPhone (optional) — Override: Enable/disable phone number detection (international)
 * @param detectCreditCard (optional) — Override: Enable/disable credit card detection
 * @param detectIban (optional) — Override: Enable/disable IBAN detection
 * @param detectAddress (optional) — Override: Enable/disable address detection (US and DE)
 * @param detectSsn (optional) — Override: Enable/disable SSN and tax ID detection
 * @param detectUrl (optional) — Override: Enable/disable URL detection
 * @param detectIp (optional) — Override: Enable/disable IP address detection
 * @param maskChar (optional) — Character used for masking (default: *)
 * @param preserveLength (optional) — If true, mask preserves original length. If false, uses mask text.
 * @param maskText (optional) — Text to use when preserve_length is false (default: [REDACTED])
 * @returns maskedText — Text with PII masked
 * @returns detectionCount — Number of PII instances detected and masked
 * @returns detections — JSON array with detection details (type, position, length)
 * @impure has side effects / drives control flow
 */
declare function processingPiiMaskRegex({ text?: string, options?: Struct, detectEmail?: bool, detectPhone?: bool, detectCreditCard?: bool, detectIban?: bool, detectAddress?: bool, detectSsn?: bool, detectUrl?: bool, detectIp?: bool, maskChar?: string, preserveLength?: bool, maskText?: string }): { maskedText: string, detectionCount: int, detections: Struct };

