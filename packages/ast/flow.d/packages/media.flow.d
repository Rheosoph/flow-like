// media — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === AI/Generative/Audio ===

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


// === Bit ===

/**
 * Loads a Bit from a string ID
 * @param bitId — Input String
 * @returns outputBit — Output Bit
 * @impure has side effects / drives control flow
 */
declare function bitFromString({ bitId: string }): Struct;

/**
 * Checks if the Bit is of the specified type and branches the execution flow accordingly.
 * @param bit — Input Bit
 * @param bitType — Type to check (e.g., "Llm", "Vlm")
 * @returns bitOut — Output Bit
 * @impure has side effects / drives control flow
 */
declare function isBitOfType({ bit: Struct, bitType: string }): Struct;

/**
 * Routes execution based on the type of the Bit
 * @param bit — Input Bit
 * @returns bitOut — Output Bit
 * @impure has side effects / drives control flow
 */
declare function switchOnBit({ bit: Struct }): Struct;


// === Data/QR ===

/**
 * Encode text as a QR code image
 * @param data — Text to encode
 * @param scale (optional) — Pixels per QR module
 * @param margin (optional) — Quiet zone in modules
 * @returns imageOut — QR code image
 * @impure has side effects / drives control flow
 */
declare function writeQrcode({ data: string, scale?: int, margin?: int }): Struct;


// === Document/DOCX ===

/**
 * Set header and/or footer text in a DOCX document
 * @param template — DOCX file
 * @param headerText (optional) — Text for the header
 * @param footerText (optional) — Text for the footer
 * @param includePageNumber (optional) — Add page number to footer
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxAddHeaderFooter({ template: Struct, headerText?: string, footerText?: string, includePageNumber?: bool, output: Struct }): Struct;

/**
 * Append a hyperlink to a DOCX document. Default color: #FF4343.
 * @param template — DOCX file
 * @param displayText — Visible link text
 * @param url — Hyperlink URL
 * @param fontColor (optional) — Link color (hex)
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxAddHyperlink({ template: Struct, displayText: string, url: string, fontColor?: string, output: Struct }): Struct;

/**
 * Insert an inline image into a DOCX document
 * @param template — DOCX file
 * @param image — Image file to insert
 * @param widthCm (optional) — Image width in cm
 * @param heightCm (optional) — Image height in cm
 * @param altText (optional) — Accessibility alt text
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxAddImage({ template: Struct, image: Struct, widthCm?: float, heightCm?: float, altText?: string, output: Struct }): Struct;

/**
 * Insert a page break into a DOCX document
 * @param template — DOCX file
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxAddPageBreak({ template: Struct, output: Struct }): Struct;

/**
 * Append a styled paragraph to a DOCX document
 * @param template — DOCX file to append to
 * @param text — Paragraph text (supports markdown bold/italic)
 * @param style (optional) — Paragraph style: Normal, Heading1-6, Title, Subtitle, Quote
 * @param fontFamily (optional) — Override font
 * @param fontSize (optional) — Override size in points (0 = use style default)
 * @param fontColor (optional) — Override text color (hex)
 * @param bold (optional) — Force bold
 * @param italic (optional) — Force italic
 * @param alignment (optional) — Text alignment: Left, Center, Right, Justify
 * @param output — Where to save
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxAddParagraph({ template: Struct, text: string, style?: string, fontFamily?: string, fontSize?: float, fontColor?: string, bold?: bool, italic?: bool, alignment?: string, output: Struct }): Struct;

/**
 * Insert a styled table from JSON data. Default: branded header with #FF4343, zebra rows.
 * @param template — DOCX file to add table to
 * @param data — JSON array of arrays (first row = headers if header_row=true)
 * @param headerRow (optional) — Style first row as header
 * @param alternateRows (optional) — Zebra striping
 * @param borderColor (optional) — Table border color (hex)
 * @param fontSize (optional) — Font size in points
 * @param output — Where to save
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxAddTable({ template: Struct, data: string, headerRow?: bool, alternateRows?: bool, borderColor?: string, fontSize?: float, output: Struct }): Struct;

/**
 * Insert a TOC field that Word will populate on open
 * @param template — DOCX file
 * @param title (optional) — TOC title
 * @param maxLevel (optional) — Maximum heading level to include (1-6)
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxAddToc({ template: Struct, title?: string, maxLevel?: int, output: Struct }): Struct;

/**
 * Create an empty DOCX with Flow Like branded theme (styled headings, Calibri font, modern spacing)
 * @param fontFamily (optional) — Default body font
 * @param fontSize (optional) — Body font size in points
 * @param themeColor (optional) — Accent color for headings (hex)
 * @param output — Where to save the DOCX file
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxCreate({ fontFamily?: string, fontSize?: float, themeColor?: string, output: Struct }): Struct;

/**
 * Extract all text content from a DOCX file as plain text
 * @param template — DOCX file to extract from
 * @returns text — Extracted text content
 * @impure has side effects / drives control flow
 */
declare function docxExtractText({ template: Struct }): string;

/**
 * Read document metadata from docProps/core.xml
 * @param template — DOCX file
 * @returns title — Document title
 * @returns author — Document author
 * @returns subject — Document subject
 * @returns keywords — Document keywords
 * @returns description — Document description
 * @impure has side effects / drives control flow
 */
declare function docxGetMetadata({ template: Struct }): { title: string, author: string, subject: string, keywords: string, description: string };

/**
 * Scan document body, headers, footers for all {{...}} placeholder strings
 * @param template — DOCX template file
 * @returns placeholders — List of placeholder strings found
 * @impure has side effects / drives control flow
 */
declare function docxListPlaceholders({ template: Struct }): string[];

/**
 * Concatenate multiple DOCX documents into one, with optional page breaks between them
 * @param documents — Array of DOCX file paths to merge in order
 * @param pageBreak (optional) — Insert page break between documents
 * @param output — Where to save the merged file
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxMerge({ documents: Struct[], pageBreak?: bool, output: Struct }): Struct;

/**
 * Remove paragraphs containing a specific placeholder. Useful for conditional content.
 * @param template — DOCX file to modify
 * @param placeholder — Text to search for — paragraphs containing this are removed
 * @param output — Where to save the result
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxRemoveParagraph({ template: Struct, placeholder: string, output: Struct }): Struct;

/**
 * Replace an image in a DOCX file by matching alt text or shape name
 * @param template — DOCX template file
 * @param identifier — Alt text or shape name of the image to replace
 * @param image — Replacement image file
 * @param scaleMode (optional) — How to handle dimensions: KeepWidth (proportional), KeepHeight (proportional), Stretch (force both, may distort), or None (use new image size)
 * @param output — Where to save the resulting DOCX file
 * @returns result — Path to the output file
 * @impure has side effects / drives control flow
 */
declare function docxReplaceImage({ template: Struct, identifier: string, image: Struct, scaleMode?: string, output: Struct }): Struct;

/**
 * Find a table containing a placeholder, duplicate that row for each data item, replacing placeholders per row
 * @param template — DOCX template file
 * @param placeholder — Placeholder in the template row (e.g. {{item}})
 * @param data — JSON array of objects — each object's keys match placeholders in the row
 * @param output — Where to save the result
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxReplaceTableRow({ template: Struct, placeholder: string, data: string, output: Struct }): Struct;

/**
 * Replace text placeholders in a DOCX template file with plain text or markdown
 * @param template — DOCX template file
 * @param placeholder — Placeholder text to find (e.g. {{name}})
 * @param replacement — Replacement text (supports markdown when enabled)
 * @param useMarkdown (optional) — Parse the replacement text as markdown
 * @param output — Where to save the resulting DOCX file
 * @returns result — Path to the output file
 * @impure has side effects / drives control flow
 */
declare function docxReplaceText({ template: Struct, placeholder: string, replacement: string, useMarkdown?: bool, output: Struct }): Struct;

/**
 * Set title, author, subject, keywords, description in document metadata
 * @param template — DOCX file
 * @param title (optional) — Document title
 * @param author (optional) — Document author
 * @param subject (optional) — Document subject
 * @param keywords (optional) — Keywords
 * @param description (optional) — Document description
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function docxSetMetadata({ template: Struct, title?: string, author?: string, subject?: string, keywords?: string, description?: string, output: Struct }): Struct;


// === Document/PDF ===

/**
 * Stamp an image at a specified position on selected PDF pages.
 * @param template — PDF file
 * @param image — Image file (PNG/JPEG)
 * @param x (optional) — X position in points
 * @param y (optional) — Y position in points
 * @param width (optional) — Image width in points
 * @param height (optional) — Image height in points
 * @param pages (optional) — Page numbers (empty = all)
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfAddImageStamp({ template: Struct, image: Struct, x?: float, y?: float, width?: float, height?: float, pages?: int[], output: Struct }): Struct;

/**
 * Add 'Page X of Y' labels to each page of a PDF.
 * @param template — PDF file
 * @param position (optional) — Position: bottom-center, bottom-right, bottom-left
 * @param fontSize (optional) — Font size in points
 * @param margin (optional) — Margin from edge in points
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfAddPageNumbers({ template: Struct, position?: string, fontSize?: float, margin?: float, output: Struct }): Struct;

/**
 * Overlay a diagonal text watermark on all pages. Default: #FF4343 at 15% opacity.
 * @param template — PDF file
 * @param text — Watermark text
 * @param fontSize (optional) — Font size in points
 * @param color (optional) — Watermark color (hex)
 * @param opacity (optional) — 0.0 to 1.0
 * @param rotationDeg (optional) — Rotation in degrees
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfAddWatermark({ template: Struct, text: string, fontSize?: float, color?: string, opacity?: float, rotationDeg?: float, output: Struct }): Struct;

/**
 * Optimize and compress a PDF by deduplicating objects and compressing streams.
 * @param template — PDF file
 * @param output — Save path
 * @returns result — Output file path
 * @returns originalSize — Size in bytes before compression
 * @returns compressedSize — Size in bytes after compression
 * @impure has side effects / drives control flow
 */
declare function pdfCompress({ template: Struct, output: Struct }): { result: Struct, originalSize: int, compressedSize: int };

/**
 * Remove password protection from a PDF using the owner or user password.
 * @param template — Encrypted PDF file
 * @param password — Owner or user password
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfDecrypt({ template: Struct, password: string, output: Struct }): Struct;

/**
 * Encrypt a PDF with a user password for restricted access.
 * @param template — PDF file
 * @param userPassword — Password required to open
 * @param ownerPassword (optional) — Password for full access (optional, defaults to user password)
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfEncrypt({ template: Struct, userPassword: string, ownerPassword?: string, output: Struct }): Struct;

/**
 * Extract specific pages (non-contiguous) from a PDF
 * @param template — PDF file
 * @param pages — Array of page numbers to extract (1-based)
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfExtractPages({ template: Struct, pages: int[], output: Struct }): Struct;

/**
 * Extract all text content from a PDF document.
 * @param template — PDF file
 * @returns text — Extracted text
 * @impure has side effects / drives control flow
 */
declare function pdfExtractText({ template: Struct }): string;

/**
 * Sets the value of a named AcroForm field in a PDF document.
 * @param template — PDF file containing form fields
 * @param fieldName — Name of the AcroForm field to fill
 * @param fieldValue — Value to set on the form field
 * @param output — Path to save the filled PDF
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfFillForm({ template: Struct, fieldName: string, fieldValue: string, output: Struct }): Struct;

/**
 * Read title, author, subject, keywords, and page count from a PDF.
 * @param template — PDF file
 * @returns title — Document title
 * @returns author — Author
 * @returns subject — Subject
 * @returns keywords — Keywords
 * @returns pageCount — Number of pages
 * @impure has side effects / drives control flow
 */
declare function pdfGetMetadata({ template: Struct }): { title: string, author: string, subject: string, keywords: string, pageCount: int };

/**
 * Reads a PDF and returns all AcroForm field names so you know which fields are available to fill.
 * @param template — PDF file containing form fields
 * @returns fieldNames — Array of all form field names in the PDF
 * @returns fieldCount — Total number of form fields
 * @impure has side effects / drives control flow
 */
declare function pdfListFormFields({ template: Struct }): { fieldNames: string[], fieldCount: int };

/**
 * Concatenate multiple PDF files into one
 * @param documents — Array of PDF file paths to merge in order
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfMerge({ documents: Struct[], output: Struct }): Struct;

/**
 * Return the number of pages in a PDF file
 * @param template — PDF file
 * @returns count — Number of pages
 * @impure has side effects / drives control flow
 */
declare function pdfPageCount({ template: Struct }): int;

/**
 * Replaces an image XObject in a PDF by name. Any image format is accepted and automatically converted to JPEG.
 * @param template — PDF file containing the image to replace
 * @param imageName — XObject image name (e.g. "Im0", "Image1")
 * @param image — Replacement image file (any format — auto-converted to JPEG)
 * @param scaleMode (optional) — How to handle dimensions: KeepWidth (proportional), KeepHeight (proportional), Stretch (force both, may distort), or None (use new image size)
 * @param output — Path to save the modified PDF
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfReplaceImage({ template: Struct, imageName: string, image: Struct, scaleMode?: string, output: Struct }): Struct;

/**
 * Attempts to find and replace text in a PDF. Best-effort: PDF text replacement may not work for all documents due to complex text encoding and fragmented content streams.
 * @param template — PDF file to modify
 * @param placeholder — Text to find in the PDF
 * @param replacement — Plain text replacement value
 * @param output — Path to save the modified PDF
 * @returns result — Output file path
 * @returns replacedCount — Number of text replacements made
 * @impure has side effects / drives control flow
 */
declare function pdfReplaceText({ template: Struct, placeholder: string, replacement: string, output: Struct }): { result: Struct, replacedCount: int };

/**
 * Rotate pages by 90, 180, or 270 degrees
 * @param template — PDF file
 * @param pages (optional) — Page numbers to rotate (1-based). Empty array = all pages.
 * @param rotation (optional) — Rotation degrees: 90, 180, or 270
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfRotatePages({ template: Struct, pages?: int[], rotation?: int, output: Struct }): Struct;

/**
 * Set title, author, subject, and keywords in a PDF's Info dictionary.
 * @param template — PDF file
 * @param title (optional) — Document title
 * @param author (optional) — Author
 * @param subject (optional) — Subject
 * @param keywords (optional) — Keywords
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfSetMetadata({ template: Struct, title?: string, author?: string, subject?: string, keywords?: string, output: Struct }): Struct;

/**
 * Extract a page range from a PDF into a new file
 * @param template — PDF file
 * @param startPage (optional) — First page to extract (1-based)
 * @param endPage (optional) — Last page to extract (1-based, inclusive)
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pdfSplit({ template: Struct, startPage?: int, endPage?: int, output: Struct }): Struct;


// === Document/PPTX ===

/**
 * Embed a simple bar chart on a PPTX slide using DrawingML chart XML.
 * @param template — PPTX file
 * @param slideNumber (optional) — 1-based slide index
 * @param chartType (optional) — bar, line, or pie
 * @param categories — Category labels
 * @param values — Numeric values
 * @param seriesName (optional) — Series label
 * @param x (optional) — X position in cm
 * @param y (optional) — Y position in cm
 * @param width (optional) — Width in cm
 * @param height (optional) — Height in cm
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pptxAddChart({ template: Struct, slideNumber?: int, chartType?: string, categories: string[], values: float[], seriesName?: string, x?: float, y?: float, width?: float, height?: float, output: Struct }): Struct;

/**
 * Place an image at a specified position on a PPTX slide.
 * @param template — PPTX file
 * @param image — Image file (PNG/JPEG)
 * @param slideNumber (optional) — 1-based slide index
 * @param x (optional) — X position in cm
 * @param y (optional) — Y position in cm
 * @param width (optional) — Width in cm
 * @param height (optional) — Height in cm
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pptxAddImageToSlide({ template: Struct, image: Struct, slideNumber?: int, x?: float, y?: float, width?: float, height?: float, output: Struct }): Struct;

/**
 * Set or replace speaker notes for a slide
 * @param template — Path to the PPTX file
 * @param slideIndex (optional) — Which slide to set notes for (1-based)
 * @param notes — Speaker notes text
 * @param output — Path where the resulting PPTX file will be saved
 * @returns result — Path to the generated PPTX file
 * @impure has side effects / drives control flow
 */
declare function pptxAddNotes({ template: Struct, slideIndex?: int, notes: string, output: Struct }): Struct;

/**
 * Add a shape (rectangle, ellipse, arrow, etc.) to a PPTX slide.
 * @param template — PPTX file
 * @param slideNumber (optional) — 1-based slide index
 * @param shape (optional) — Shape preset: rect, ellipse, roundRect, rightArrow, diamond, triangle
 * @param x (optional) — X position in cm
 * @param y (optional) — Y position in cm
 * @param width (optional) — Width in cm
 * @param height (optional) — Height in cm
 * @param fillColor (optional) — Fill hex color
 * @param lineColor (optional) — Outline hex color (empty = no outline)
 * @param text (optional) — Optional text inside shape
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pptxAddShape({ template: Struct, slideNumber?: int, shape?: string, x?: float, y?: float, width?: float, height?: float, fillColor?: string, lineColor?: string, text?: string, output: Struct }): Struct;

/**
 * Add a blank slide to a PPTX presentation.
 * @param template — PPTX file
 * @param output — Save path
 * @returns result — Output file path
 * @returns slideNumber — New slide's index (1-based)
 * @impure has side effects / drives control flow
 */
declare function pptxAddSlide({ template: Struct, output: Struct }): { result: Struct, slideNumber: int };

/**
 * Add a branded table to a PPTX slide. Header row uses #FF4343 with white text.
 * @param template — PPTX file
 * @param slideNumber (optional) — 1-based slide index
 * @param headers — Column headers
 * @param rows — Table data as JSON array of arrays
 * @param x (optional) — X position in cm
 * @param y (optional) — Y position in cm
 * @param width (optional) — Table width in cm
 * @param rowHeight (optional) — Row height in cm
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pptxAddTableToSlide({ template: Struct, slideNumber?: int, headers: string[], rows: string[], x?: float, y?: float, width?: float, rowHeight?: float, output: Struct }): Struct;

/**
 * Add a styled text box to a specific slide in a PPTX.
 * @param template — PPTX file
 * @param slideNumber (optional) — 1-based slide index
 * @param text — Text content
 * @param x (optional) — X position in cm
 * @param y (optional) — Y position in cm
 * @param width (optional) — Width in cm
 * @param height (optional) — Height in cm
 * @param fontSize (optional) — Font size in points
 * @param fontColor (optional) — Hex color
 * @param bold (optional) — Bold text
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pptxAddTextBox({ template: Struct, slideNumber?: int, text: string, x?: float, y?: float, width?: float, height?: float, fontSize?: float, fontColor?: string, bold?: bool, output: Struct }): Struct;

/**
 * Create a blank PPTX presentation with Flow Like brand theme (16:9, Calibri, #FF4343 accent).
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pptxCreate({ output: Struct }): Struct;

/**
 * Remove a slide at the given index from a PPTX file
 * @param template — Path to the PPTX file
 * @param slideIndex (optional) — Index of the slide to delete (1-based)
 * @param output — Path where the resulting PPTX file will be saved
 * @returns result — Path to the generated PPTX file
 * @impure has side effects / drives control flow
 */
declare function pptxDeleteSlide({ template: Struct, slideIndex?: int, output: Struct }): Struct;

/**
 * Clone a slide at a given index, inserting the copy at a target position. Preserves formatting, layouts, and master references.
 * @param template — Path to the PPTX file
 * @param slideIndex (optional) — Index of the slide to clone (1-based)
 * @param targetIndex (optional) — Position to insert the cloned slide (1-based)
 * @param output — Path where the resulting PPTX file will be saved
 * @returns result — Path to the generated PPTX file
 * @impure has side effects / drives control flow
 */
declare function pptxDuplicateSlide({ template: Struct, slideIndex?: int, targetIndex?: int, output: Struct }): Struct;

/**
 * Extract all text content from all slides as plain text
 * @param template — Path to the PPTX file
 * @returns text — Extracted text from all slides
 * @impure has side effects / drives control flow
 */
declare function pptxExtractText({ template: Struct }): string;

/**
 * Read presentation metadata (title, author, subject, keywords)
 * @param template — PPTX file
 * @returns title — Document title
 * @returns author — Document author
 * @returns subject — Document subject
 * @returns keywords — Document keywords
 * @impure has side effects / drives control flow
 */
declare function pptxGetMetadata({ template: Struct }): { title: string, author: string, subject: string, keywords: string };

/**
 * Scan all slides for {{...}} placeholder strings
 * @param template — Path to the PPTX file
 * @returns placeholders — List of unique placeholder names found
 * @impure has side effects / drives control flow
 */
declare function pptxListPlaceholders({ template: Struct }): string[];

/**
 * Combine slides from multiple PPTX files into one. The base file's theme and masters are preserved.
 * @param base — Base PPTX file (theme/masters kept)
 * @param additional — Additional PPTX files to merge (array of paths)
 * @param output — Where to save the merged file
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pptxMerge({ base: Struct, additional: Struct[], output: Struct }): Struct;

/**
 * Move a slide from one position to another
 * @param template — Path to the PPTX file
 * @param fromIndex (optional) — Current position of the slide (1-based)
 * @param toIndex (optional) — Target position for the slide (1-based)
 * @param output — Path where the resulting PPTX file will be saved
 * @returns result — Path to the generated PPTX file
 * @impure has side effects / drives control flow
 */
declare function pptxReorderSlides({ template: Struct, fromIndex?: int, toIndex?: int, output: Struct }): Struct;

/**
 * Replaces images in a PowerPoint (PPTX) file by matching alt text or shape name
 * @param template — Path to the PPTX template file
 * @param identifier — Alt text or shape name of the image to replace
 * @param image — Path to the replacement image file
 * @param scaleMode (optional) — How to handle dimensions: KeepWidth (proportional), KeepHeight (proportional), Stretch (force both, may distort), or None (use new image size)
 * @param output — Path where the resulting PPTX file will be saved
 * @returns result — Path to the generated PPTX file
 * @impure has side effects / drives control flow
 */
declare function pptxReplaceImage({ template: Struct, identifier: string, image: Struct, scaleMode?: string, output: Struct }): Struct;

/**
 * Populate a table on a slide that contains a placeholder in its first cell with structured data (JSON array of arrays). Inherits the table's existing styling.
 * @param template — Path to the PPTX file
 * @param slideIndex (optional) — Which slide contains the table (1-based)
 * @param placeholder — Placeholder text to find in the table
 * @param data — JSON array of arrays with table data
 * @param hasHeader (optional) — Whether the first row of data is a header row
 * @param output — Path where the resulting PPTX file will be saved
 * @returns result — Path to the generated PPTX file
 * @impure has side effects / drives control flow
 */
declare function pptxReplaceTableData({ template: Struct, slideIndex?: int, placeholder: string, data: string, hasHeader?: bool, output: Struct }): Struct;

/**
 * Replaces text placeholders in a PowerPoint (PPTX) file with plain or markdown-formatted text
 * @param template — Path to the PPTX template file
 * @param placeholder — The placeholder text to find in the template
 * @param replacement — The replacement text (supports markdown when enabled)
 * @param useMarkdown (optional) — Parse replacement text as markdown for rich formatting
 * @param output — Path where the resulting PPTX file will be saved
 * @returns result — Path to the generated PPTX file
 * @impure has side effects / drives control flow
 */
declare function pptxReplaceText({ template: Struct, placeholder: string, replacement: string, useMarkdown?: bool, output: Struct }): Struct;

/**
 * Set title, author, subject, keywords in presentation metadata
 * @param template — PPTX file
 * @param title (optional) — Document title
 * @param author (optional) — Document author
 * @param subject (optional) — Document subject
 * @param keywords (optional) — Comma-separated keywords
 * @param output — Save path
 * @returns result — Output file path
 * @impure has side effects / drives control flow
 */
declare function pptxSetMetadata({ template: Struct, title?: string, author?: string, subject?: string, keywords?: string, output: Struct }): Struct;

/**
 * Return the number of slides in a PPTX file
 * @param template — Path to the PPTX file
 * @returns count — Number of slides in the presentation
 * @impure has side effects / drives control flow
 */
declare function pptxSlideCount({ template: Struct }): int;


// === Image/Annotate ===

/**
 * Draw Bounding Boxes
 * @param imageIn — Image object
 * @param bboxes — Bounding Boxes
 * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
 * @returns imageOut — Image with Bounding Boxes
 * @impure has side effects / drives control flow
 */
declare function drawBoxes({ imageIn: Struct, bboxes: Struct[], useRef?: bool }): Struct;

/**
 * Make Bounding Box
 * @param definition (optional) — Bounding Box Definition
 * @param classIdx (optional) — Class Index
 * @param score (optional) — Score or Confidence
 * @param x1 — Left
 * @param y1 — Top
 * @param x2 — Right
 * @param y2 — Bottom
 * @returns bbox — Bounding Boxes
 * @impure has side effects / drives control flow
 */
declare function makeBoxe({ definition?: string, classIdx?: int, score?: float, x1: float, y1: float, x2: float, y2: float }): Struct;


// === Image/Content ===

/**
 * Read/Decode QR Codes and Barcodes
 * @param imageIn — Image object
 * @param filter (optional) — Filter for Certain Code Type
 * @returns results — Detected/Decoded Codes
 * @impure has side effects / drives control flow
 */
declare function readBarcodes({ imageIn: Struct, filter?: bool }): Struct[];

/**
 * Read image from path
 * @param path — FlowPath
 * @param applyExif (optional) — Apply Exif Orientation
 * @returns imageOut — Image object
 * @impure has side effects / drives control flow
 */
declare function readImage({ path: Struct, applyExif?: bool }): Struct;

/**
 * Read image from path
 * @param signedUrl — Signed Url
 * @param applyExif (optional) — Apply Exif Orientation
 * @returns imageOut — Image object
 * @impure has side effects / drives control flow
 */
declare function readImageUrl({ signedUrl: string, applyExif?: bool }): Struct;

/**
 * Write image to path
 * @param imageIn — The image to write to path
 * @param path — FlowPath
 * @param type (optional) — Image Type
 * @param quality (optional) — Encoding Quality
 * @impure has side effects / drives control flow
 */
declare function writeImage({ imageIn: Struct, path: Struct, type?: string, quality?: int }): void;


// === Image/Metadata ===

/**
 * Get Image Dimensions
 * @param imageIn — Image object
 * @returns width — Image Width
 * @returns height — Image Height
 * @impure has side effects / drives control flow
 */
declare function getDimensions({ imageIn: Struct }): { width: int, height: int };


// === Image/Overlay ===

/**
 * Overlay one image on top of another with configurable position, size, opacity and fit mode
 * @param baseImage — The background image
 * @param overlayImage — The image to overlay on top
 * @param useRef (optional) — Use reference of the base image, transforming the original instead of a copy
 * @param x (optional) — Horizontal offset in pixels from the left edge
 * @param y (optional) — Vertical offset in pixels from the top edge
 * @param maxW (optional) — Maximum width of the overlay (0 = original width)
 * @param maxH (optional) — Maximum height of the overlay (0 = original height)
 * @param opacity (optional) — Overlay opacity from 0.0 (transparent) to 1.0 (opaque)
 * @param fitMode (optional) — How to fit the overlay into max width/height
 * @returns imageOut — Result image with overlay applied
 * @impure has side effects / drives control flow
 */
declare function imageOverlay({ baseImage: Struct, overlayImage: Struct, useRef?: bool, x?: int, y?: int, maxW?: int, maxH?: int, opacity?: float, fitMode?: string }): Struct;

/**
 * Draw text on top of an image with configurable font size, position, and color
 * @param baseImage — The background image to draw text on
 * @param text (optional) — The text string to render
 * @param useRef (optional) — Use reference of the base image, transforming the original instead of a copy
 * @param x (optional) — Horizontal offset in pixels from the left edge
 * @param y (optional) — Vertical offset in pixels from the top edge
 * @param fontSize (optional) — Font size in pixels
 * @param color (optional) — Text color as hex string (e.g. #FF0000 or #FF0000AA for alpha)
 * @returns imageOut — Result image with text rendered
 * @impure has side effects / drives control flow
 */
declare function textOverlay({ baseImage: Struct, text?: string, useRef?: bool, x?: int, y?: int, fontSize?: float, color?: string }): Struct;


// === Image/PDF ===

/**
 * Count pages in a PDF
 * @param pdf — PDF file
 * @returns pageCount — Page count
 * @impure has side effects / drives control flow
 */
declare function pdfPageCount({ pdf: Struct }): int;

/**
 * Render a single PDF page as an image
 * @param pdf — PDF file
 * @param page (optional) — 1-based page number
 * @param scale (optional) — Render scale
 * @param bgColor (optional) — Background color for the rendered page
 * @returns image — Rendered image
 * @impure has side effects / drives control flow
 */
declare function pdfPageToImage({ pdf: Struct, page?: int, scale?: float, bgColor?: string }): Struct;

/**
 * Render every PDF page as an ordered image array
 * @param pdf — PDF file
 * @param scale (optional) — Render scale
 * @param bgColor (optional) — Background color for rendered pages
 * @returns images — Rendered images
 * @impure has side effects / drives control flow
 */
declare function pdfToImages({ pdf: Struct, scale?: float, bgColor?: string }): Struct[];


// === Image/Transform ===

/**
 * Adjust Image Contrast
 * @param imageIn — Image object
 * @param contrast — Contrast
 * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
 * @returns imageOut — Image with Applied Contrast
 * @impure has side effects / drives control flow
 */
declare function contrastImage({ imageIn: Struct, contrast: float, useRef?: bool }): Struct;

/**
 * Convert Image Color/Pixel Type (e.g. to Grayscale)
 * @param imageIn — Image object
 * @param pixelType (optional) — Target Pixel Type
 * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
 * @returns imageOut — Image with Target Color/Pixel Type
 * @impure has side effects / drives control flow
 */
declare function convertImage({ imageIn: Struct, pixelType?: string, useRef?: bool }): Struct;

/**
 * Crop Image
 * @param imageIn — Image object
 * @param bbox — Bounding Box
 * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
 * @returns imageOut — Cropped Image object
 * @impure has side effects / drives control flow
 */
declare function cropImage({ imageIn: Struct, bbox: Struct, useRef?: bool }): Struct;

/**
 * Resize Image
 * @param imageIn — Image object
 * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
 * @param mode (optional) — Resize Mode
 * @param filter (optional) — Resize Filter Algorithm
 * @param widthIn (optional) — Resized Image Target Width
 * @param heightIn (optional) — Resized Image Target Height
 * @returns imageOut — Image object
 * @returns widthOut — Resized Image Result Width
 * @returns heightOut — Resized Image Result Height
 * @impure has side effects / drives control flow
 */
declare function resizeImage({ imageIn: Struct, useRef?: bool, mode?: string, filter?: string, widthIn?: int, heightIn?: int }): { imageOut: Struct, widthOut: int, heightOut: int };


// === Web/Camera ===

/**
 * Writes an image to a data URL
 * @param image — The image to write to a data URL
 * @param format (optional) — The format of the image (e.g., png, jpeg)
 * @returns url — The data URL of the written image
 * @impure has side effects / drives control flow
 */
declare function imageWriteDataurl({ image: Struct, format?: string }): string;

