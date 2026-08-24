// media — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace ai {
    // === Bit ===

    /**
     * Checks if the Bit is of the specified type and branches the execution flow accordingly.
     * @node is_bit_of_type @alias isBitOfType
     * @param bit — Input Bit
     * @param bitType — Type to check (e.g., "Llm", "Vlm")
     * @returns bitOut — Output Bit
     * @impure has side effects / drives control flow
     */
    function isBitOfType({ bit: Struct, bitType: string }): Struct;

    /**
     * Loads a Bit from a string ID
     * @node bit_from_string @alias bitFromString
     * @param bitId — Input String
     * @returns outputBit — Output Bit
     * @impure has side effects / drives control flow
     */
    function loadBit({ bitId: string }): Struct;

    /**
     * Routes execution based on the type of the Bit
     * @node switch_on_bit @alias switchOnBit
     * @param bit — Input Bit
     * @returns bitOut — Output Bit
     * @impure has side effects / drives control flow
     */
    function switchOnBit({ bit: Struct }): Struct;

    namespace audio {
        // === AI/Generative/Audio ===

        /**
         * Transcribes audio locally with an installed any-speech-to-text model bit. Decodes WAV, MP3, FLAC, OGG (Vorbis/Opus), WebM/Opus, M4A/MP4 (AAC) and PCM, including browser MediaRecorder output (Chrome WebM/Opus, Safari MP4/AAC).
         * @node ai_audio_local_speech_to_text @alias aiAudioLocalSpeechToText
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
        function localSpeechToText({ bit: Struct, audio: Struct, language?: string, task?: string, timestamps?: bool }): { text: string, message: Struct, history: Struct, metadata: Struct };

        /**
         * Generates WAV speech locally with an installed any-tts model bit.
         * @node ai_audio_local_text_to_speech @alias aiAudioLocalTextToSpeech
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
        function localTextToSpeech({ bit: Struct, text?: string, outputPath: Struct, language?: string, voice?: string, instruct?: string, maxTokens?: int, temperature?: float, cfgScale?: float, referenceAudio?: Struct }): { path: Struct, metadata: Struct };

        /**
         * Transcribes or translates audio with an existing provider Bit.
         * @node ai_audio_speech_to_text @alias aiAudioSpeechToText
         * @param provider — Existing provider Bit
         * @param audio — Audio FlowPath
         * @param providerOptions (optional) — Typed provider-specific speech-to-text options
         * @returns text — Transcript text
         * @returns message — Transcript as a user HistoryMessage
         * @returns history — Transcript wrapped in History
         * @returns metadata — Transcription metadata
         * @impure has side effects / drives control flow
         */
        function speechToText({ provider: Struct, audio: Struct, providerOptions?: Struct }): { text: string, message: Struct, history: Struct, metadata: Struct };

        /**
         * Generates speech audio with an existing provider Bit and writes it to FlowPath.
         * @node ai_audio_text_to_speech @alias aiAudioTextToSpeech
         * @param provider — Existing provider Bit
         * @param text (optional) — Text to synthesize
         * @param outputPath — Destination FlowPath for generated audio
         * @param providerOptions (optional) — Typed provider-specific text-to-speech options
         * @returns path — Generated audio path
         * @returns metadata — Generation metadata
         * @impure has side effects / drives control flow
         */
        function textToSpeech({ provider: Struct, text?: string, outputPath: Struct, providerOptions?: Struct }): { path: Struct, metadata: Struct };

        namespace options {
            // === AI/Generative/Audio/Options ===

            /**
             * Creates typed speech-to-text options for Gemini and Vertex audio transcription.
             * @node ai_audio_stt_options_google @alias aiAudioSttOptionsGoogle
             * @param prompt (optional) — Transcription instruction prompt
             * @returns options — Typed speech-to-text provider options
             */
            function sttGoogle({ prompt?: string }): Struct;

            /**
             * Creates typed speech-to-text options for OpenAI-compatible providers.
             * @node ai_audio_stt_options_openai_compatible @alias aiAudioSttOptionsOpenaiCompatible
             * @param prompt (optional) — Optional transcription prompt or context
             * @param language (optional) — Optional source language code
             * @param responseFormat (optional) — Provider response format
             * @param translate (optional) — Translate audio to English when the provider supports it
             * @returns options — Typed speech-to-text provider options
             */
            function sttOpenaiCompatible({ prompt?: string, language?: string, responseFormat?: string, translate?: bool }): Struct;

            /**
             * Creates typed speech-to-text options for xAI transcription models.
             * @node ai_audio_stt_options_xai @alias aiAudioSttOptionsXai
             * @param prompt (optional) — Optional transcription prompt or context
             * @param language (optional) — Optional source language code
             * @returns options — Typed speech-to-text provider options
             */
            function sttXai({ prompt?: string, language?: string }): Struct;

            /**
             * Creates typed text-to-speech options for Gemini and Vertex speech models.
             * @node ai_audio_tts_options_google @alias aiAudioTtsOptionsGoogle
             * @param voice (optional) — Google prebuilt voice name
             * @param instructions (optional) — Optional style or delivery instructions
             * @param language (optional) — Optional BCP-47 language code
             * @param outputFormat (optional) — Requested output audio format
             * @returns options — Typed text-to-speech provider options
             */
            function ttsGoogle({ voice?: string, instructions?: string, language?: string, outputFormat?: string }): Struct;

            /**
             * Creates typed text-to-speech options for Hugging Face speech models.
             * @node ai_audio_tts_options_huggingface @alias aiAudioTtsOptionsHuggingface
             * @param voice (optional) — Optional voice parameter
             * @param outputFormat (optional) — Requested output audio format
             * @param speed (optional) — Playback speed multiplier. Use 0 for provider default.
             * @returns options — Typed text-to-speech provider options
             */
            function ttsHuggingface({ voice?: string, outputFormat?: string, speed?: float }): Struct;

            /**
             * Creates typed text-to-speech options for Mistral speech models.
             * @node ai_audio_tts_options_mistral @alias aiAudioTtsOptionsMistral
             * @param voice (optional) — Mistral voice identifier
             * @param outputFormat (optional) — Requested output audio format
             * @returns options — Typed text-to-speech provider options
             */
            function ttsMistral({ voice?: string, outputFormat?: string }): Struct;

            /**
             * Creates typed text-to-speech options for OpenAI-compatible providers.
             * @node ai_audio_tts_options_openai_compatible @alias aiAudioTtsOptionsOpenaiCompatible
             * @param voice (optional) — Provider voice identifier
             * @param instructions (optional) — Optional style or delivery instructions
             * @param outputFormat (optional) — Requested output audio format
             * @param speed (optional) — Playback speed multiplier. Use 0 for provider default.
             * @returns options — Typed text-to-speech provider options
             */
            function ttsOpenaiCompatible({ voice?: string, instructions?: string, outputFormat?: string, speed?: float }): Struct;

            /**
             * Creates typed text-to-speech options for xAI speech models.
             * @node ai_audio_tts_options_xai @alias aiAudioTtsOptionsXai
             * @param voice (optional) — xAI voice identifier
             * @param language (optional) — Optional language code
             * @param outputFormat (optional) — Requested output audio codec
             * @param sampleRate (optional) — Optional output sample rate. Use 0 for provider default.
             * @param bitRate (optional) — Optional MP3 bit rate. Use 0 for provider default.
             * @returns options — Typed text-to-speech provider options
             */
            function ttsXai({ voice?: string, language?: string, outputFormat?: string, sampleRate?: int, bitRate?: int }): Struct;
        }
    }

    namespace image {
        // === AI/Generative/Image ===

        /**
         * Generates one image with an existing provider Bit and writes it to FlowPath.
         * @node ai_image_generate @alias aiImageGenerate
         * @param provider — Existing provider Bit
         * @param history — Conversation history. The final user message is used as the image prompt.
         * @param outputPath — Destination FlowPath for generated image output
         * @param providerOptions (optional) — Typed provider-specific image options
         * @returns path — First generated image path
         * @returns paths — All generated image paths
         * @returns metadata — Generation metadata
         * @impure has side effects / drives control flow
         */
        function generate({ provider: Struct, history: Struct, outputPath: Struct, providerOptions?: Struct }): { path: Struct, paths: Struct[], metadata: Struct };

        namespace options {
            // === AI/Generative/Image/Options ===

            /**
             * Creates typed image options for AWS Bedrock image models.
             * @node ai_image_options_aws_bedrock @alias aiImageOptionsAwsBedrock
             * @param aspectRatio (optional) — Bedrock image aspect ratio. Ignored when Size is set.
             * @param size (optional) — Bedrock output size
             * @param quality (optional) — Bedrock image quality
             * @param negativePrompt (optional) — Text describing what to avoid
             * @param seed (optional) — Optional seed. Use 0 for provider default.
             * @param outputFormat (optional) — Requested output image format
             * @returns options — Typed image generation provider options
             */
            function awsBedrock({ aspectRatio?: string, size?: string, quality?: string, negativePrompt?: string, seed?: int, outputFormat?: string }): Struct;

            /**
             * Creates typed image options for Google AI Studio and Vertex Imagen models.
             * @node ai_image_options_google_imagen @alias aiImageOptionsGoogleImagen
             * @param aspectRatio (optional) — Imagen aspect ratio
             * @param negativePrompt (optional) — Text describing what to avoid
             * @param seed (optional) — Optional seed. Use 0 for provider default.
             * @param outputFormat (optional) — Requested output image format
             * @returns options — Typed image generation provider options
             */
            function googleImagen({ aspectRatio?: string, negativePrompt?: string, seed?: int, outputFormat?: string }): Struct;

            /**
             * Creates typed image options for Hugging Face text-to-image models.
             * @node ai_image_options_huggingface @alias aiImageOptionsHuggingface
             * @param size (optional) — Hugging Face output size
             * @param negativePrompt (optional) — Text describing what to avoid
             * @param seed (optional) — Optional seed. Use 0 for provider default.
             * @param outputFormat (optional) — Requested output image format
             * @returns options — Typed image generation provider options
             */
            function huggingface({ size?: string, negativePrompt?: string, seed?: int, outputFormat?: string }): Struct;

            /**
             * Creates typed image options for OpenAI and Azure OpenAI image generation.
             * @node ai_image_options_openai @alias aiImageOptionsOpenai
             * @param size (optional) — OpenAI image size
             * @param quality (optional) — OpenAI image quality
             * @param background (optional) — OpenAI background behavior
             * @param outputFormat (optional) — Requested output image format
             * @returns options — Typed image generation provider options
             */
            function openai({ size?: string, quality?: string, background?: string, outputFormat?: string }): Struct;

            /**
             * Creates typed image options for OpenRouter image-output models.
             * @node ai_image_options_openrouter @alias aiImageOptionsOpenrouter
             * @param aspectRatio (optional) — OpenRouter image aspect ratio
             * @param size (optional) — OpenRouter image size
             * @returns options — Typed image generation provider options
             */
            function openrouter({ aspectRatio?: string, size?: string }): Struct;

            /**
             * Creates typed image options for Together text-to-image models.
             * @node ai_image_options_together @alias aiImageOptionsTogether
             * @param aspectRatio (optional) — Together aspect ratio. Ignored when Size is set.
             * @param size (optional) — Together output size
             * @param negativePrompt (optional) — Text describing what to avoid
             * @param seed (optional) — Optional seed. Use 0 for provider default.
             * @param outputFormat (optional) — Requested output image format
             * @returns options — Typed image generation provider options
             */
            function together({ aspectRatio?: string, size?: string, negativePrompt?: string, seed?: int, outputFormat?: string }): Struct;

            /**
             * Creates typed image options for xAI image generation.
             * @node ai_image_options_xai @alias aiImageOptionsXai
             * @param aspectRatio (optional) — xAI image aspect ratio
             * @returns options — Typed image generation provider options
             */
            function xai({ aspectRatio?: string }): Struct;
        }
    }

    namespace video {
        // === AI/Generative/Video ===

        /**
         * Generates video with an existing provider Bit and writes it to FlowPath.
         * @node ai_video_generate @alias aiVideoGenerate
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
        function generate({ provider: Struct, prompt?: string, outputPath: Struct, firstFrame: Struct, lastFrame: Struct, inputVideo: Struct, providerOptions?: Struct }): { path: Struct, paths: Struct[], metadata: Struct };

        namespace options {
            // === AI/Generative/Video/Options ===

            /**
             * Creates typed video options for fal.ai video models.
             * @node ai_video_options_fal @alias aiVideoOptionsFal
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
            function fal({ negativePrompt?: string, aspectRatio?: string, size?: string, durationSeconds?: int, seed?: int, generateAudio?: bool, pollIntervalSeconds?: int, maxWaitSeconds?: int }): Struct;

            /**
             * Creates typed video options for OpenAI Sora models.
             * @node ai_video_options_openai_sora @alias aiVideoOptionsOpenaiSora
             * @param size (optional) — Sora video size
             * @param durationSeconds (optional) — Requested duration in seconds. Use 0 for provider default.
             * @param pollIntervalSeconds (optional) — Seconds between provider status checks
             * @param maxWaitSeconds (optional) — Maximum seconds to wait for completion
             * @returns options — Typed video generation provider options
             */
            function openaiSora({ size?: string, durationSeconds?: int, pollIntervalSeconds?: int, maxWaitSeconds?: int }): Struct;

            /**
             * Creates typed video options for Replicate video models.
             * @node ai_video_options_replicate @alias aiVideoOptionsReplicate
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
            function replicate({ negativePrompt?: string, aspectRatio?: string, size?: string, durationSeconds?: int, seed?: int, generateAudio?: bool, pollIntervalSeconds?: int, maxWaitSeconds?: int }): Struct;

            /**
             * Creates typed video options for Runway models.
             * @node ai_video_options_runway @alias aiVideoOptionsRunway
             * @param aspectRatio (optional) — Runway aspect ratio
             * @param size (optional) — Runway output size
             * @param durationSeconds (optional) — Requested duration in seconds. Use 0 for provider default.
             * @param seed (optional) — Optional deterministic seed. Use 0 for provider default.
             * @param pollIntervalSeconds (optional) — Seconds between provider status checks
             * @param maxWaitSeconds (optional) — Maximum seconds to wait for completion
             * @returns options — Typed video generation provider options
             */
            function runway({ aspectRatio?: string, size?: string, durationSeconds?: int, seed?: int, pollIntervalSeconds?: int, maxWaitSeconds?: int }): Struct;

            /**
             * Creates typed video options for Google Vertex Veo models.
             * @node ai_video_options_vertex_veo @alias aiVideoOptionsVertexVeo
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
            function vertexVeo({ negativePrompt?: string, aspectRatio?: string, size?: string, durationSeconds?: int, seed?: int, count?: int, pollIntervalSeconds?: int, maxWaitSeconds?: int }): Struct;
        }

        namespace provider {
            // === AI/Generative/Video/Provider ===

            /**
             * Builds a fal.ai queued video generation provider Bit.
             * @node ai_video_build_fal @alias aiVideoBuildFal
             * @param apiKey (optional) — fal API key
             * @param endpoint (optional) — fal queue endpoint
             * @param modelId (optional) — fal model path
             * @returns provider — Bit containing the video generation provider configuration
             * @impure has side effects / drives control flow
             */
            function fal({ apiKey?: string, endpoint?: string, modelId?: string }): Struct;

            /**
             * Builds a Replicate video generation provider Bit.
             * @node ai_video_build_replicate @alias aiVideoBuildReplicate
             * @param apiKey (optional) — Replicate API token
             * @param endpoint (optional) — Replicate API endpoint
             * @param modelId (optional) — Replicate owner/model path for official models
             * @param version (optional) — Optional model version hash for community predictions
             * @returns provider — Bit containing the video generation provider configuration
             * @impure has side effects / drives control flow
             */
            function replicate({ apiKey?: string, endpoint?: string, modelId?: string, version?: string }): Struct;

            /**
             * Builds a Runway video generation provider Bit.
             * @node ai_video_build_runway @alias aiVideoBuildRunway
             * @param apiKey (optional) — Runway API key
             * @param endpoint (optional) — Runway API endpoint
             * @param apiVersion (optional) — Runway API version header
             * @param modelId (optional) — Runway video model ID
             * @returns provider — Bit containing the video generation provider configuration
             * @impure has side effects / drives control flow
             */
            function runway({ apiKey?: string, endpoint?: string, apiVersion?: string, modelId?: string }): Struct;
        }
    }
}

declare namespace audio {
    // === Audio ===

    /**
     * Decode audio and report waveform, peak/RMS, and silence ranges
     * @node video_analyze_audio @alias videoAnalyzeAudio
     * @param source — Source audio/media FlowPath
     * @param waveformBuckets (optional) — Number of waveform buckets
     * @param silenceThresholdDb (optional) — RMS threshold in dB
     * @param windowMs (optional) — Silence analysis window
     * @param minSilenceMs (optional) — Minimum silence duration
     * @returns report — Audio analysis report
     * @returns waveform — Waveform buckets
     * @returns silence — Detected silence ranges
     * @impure has side effects / drives control flow
     */
    function analyze({ source: Struct, waveformBuckets?: int, silenceThresholdDb?: float, windowMs?: int, minSilenceMs?: int }): { report: Struct, waveform: Struct[], silence: Struct[] };

    /**
     * Decode audio and return silence intervals
     * @node video_detect_silence @alias videoDetectSilence
     * @param source — Source audio/media FlowPath
     * @param silenceThresholdDb (optional) — RMS threshold in dB
     * @param windowMs (optional) — Silence analysis window
     * @param minSilenceMs (optional) — Minimum silence duration
     * @returns silence — Detected silence ranges
     * @returns count — Detected silence range count
     * @impure has side effects / drives control flow
     */
    function detectSilence({ source: Struct, silenceThresholdDb?: float, windowMs?: int, minSilenceMs?: int }): { silence: Struct[], count: int };

    /**
     * Decode an audio/media object and write WAV PCM output
     * @node video_audio_to_wav @alias videoAudioToWav
     * @param source — Source audio/media FlowPath
     * @param target — Target WAV FlowPath
     * @param audioTrackId (optional) — Audio track id, or 0 for default
     * @returns result — Written WAV FlowPath
     * @returns report — Audio conversion report
     * @impure has side effects / drives control flow
     */
    function toWav({ source: Struct, target: Struct, audioTrackId?: int }): { result: Struct, report: Struct };

    /**
     * Decode audio, apply gain/normalization/fades, and write WAV PCM output
     * @node video_transform_audio @alias videoTransformAudio
     * @param source — Source audio/media FlowPath
     * @param target — Target WAV FlowPath
     * @param audioTrackId (optional) — Audio track id, or 0 for default
     * @param gainFactor (optional) — Linear gain factor
     * @param gainDb (optional) — Gain in decibels
     * @param normalizePeak (optional) — Target peak amplitude, or 0 to skip
     * @param fadeInSeconds (optional) — Fade-in seconds
     * @param fadeOutSeconds (optional) — Fade-out seconds
     * @param fadeShape (optional) — linear or equal_power
     * @returns result — Written WAV FlowPath
     * @returns report — Audio transform report
     * @impure has side effects / drives control flow
     */
    function transform({ source: Struct, target: Struct, audioTrackId?: int, gainFactor?: float, gainDb?: float, normalizePeak?: float, fadeInSeconds?: float, fadeOutSeconds?: float, fadeShape?: string }): { result: Struct, report: Struct };
}

declare namespace docx {
    // === Document/DOCX ===

    /**
     * Set header and/or footer text in a DOCX document
     * @node docx_add_header_footer @alias docxAddHeaderFooter
     * @param template — DOCX file
     * @param headerText (optional) — Text for the header
     * @param footerText (optional) — Text for the footer
     * @param includePageNumber (optional) — Add page number to footer
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addHeaderFooter({ template: Struct, headerText?: string, footerText?: string, includePageNumber?: bool, output: Struct }): Struct;

    /**
     * Append a hyperlink to a DOCX document. Default color: #FF4343.
     * @node docx_add_hyperlink @alias docxAddHyperlink
     * @param template — DOCX file
     * @param displayText — Visible link text
     * @param url — Hyperlink URL
     * @param fontColor (optional) — Link color (hex)
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addHyperlink({ template: Struct, displayText: string, url: string, fontColor?: string, output: Struct }): Struct;

    /**
     * Insert an inline image into a DOCX document
     * @node docx_add_image @alias docxAddImage
     * @param template — DOCX file
     * @param image — Image file to insert
     * @param widthCm (optional) — Image width in cm
     * @param heightCm (optional) — Image height in cm
     * @param altText (optional) — Accessibility alt text
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addImage({ template: Struct, image: Struct, widthCm?: float, heightCm?: float, altText?: string, output: Struct }): Struct;

    /**
     * Insert a page break into a DOCX document
     * @node docx_add_page_break @alias docxAddPageBreak
     * @param template — DOCX file
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addPageBreak({ template: Struct, output: Struct }): Struct;

    /**
     * Append a styled paragraph to a DOCX document
     * @node docx_add_paragraph @alias docxAddParagraph
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
    function addParagraph({ template: Struct, text: string, style?: string, fontFamily?: string, fontSize?: float, fontColor?: string, bold?: bool, italic?: bool, alignment?: string, output: Struct }): Struct;

    /**
     * Insert a styled table from JSON data. Default: branded header with #FF4343, zebra rows.
     * @node docx_add_table @alias docxAddTable
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
    function addTable({ template: Struct, data: string, headerRow?: bool, alternateRows?: bool, borderColor?: string, fontSize?: float, output: Struct }): Struct;

    /**
     * Insert a TOC field that Word will populate on open
     * @node docx_add_toc @alias docxAddToc
     * @param template — DOCX file
     * @param title (optional) — TOC title
     * @param maxLevel (optional) — Maximum heading level to include (1-6)
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addToc({ template: Struct, title?: string, maxLevel?: int, output: Struct }): Struct;

    /**
     * Create an empty DOCX with Flow Like branded theme (styled headings, Calibri font, modern spacing)
     * @node docx_create @alias docxCreate
     * @param fontFamily (optional) — Default body font
     * @param fontSize (optional) — Body font size in points
     * @param themeColor (optional) — Accent color for headings (hex)
     * @param output — Where to save the DOCX file
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function create({ fontFamily?: string, fontSize?: float, themeColor?: string, output: Struct }): Struct;

    /**
     * Extract all text content from a DOCX file as plain text
     * @node docx_extract_text @alias docxExtractText
     * @param template — DOCX file to extract from
     * @returns text — Extracted text content
     * @impure has side effects / drives control flow
     */
    function extractText({ template: Struct }): string;

    /**
     * Read document metadata from docProps/core.xml
     * @node docx_get_metadata @alias docxGetMetadata
     * @param template — DOCX file
     * @returns title — Document title
     * @returns author — Document author
     * @returns subject — Document subject
     * @returns keywords — Document keywords
     * @returns description — Document description
     * @impure has side effects / drives control flow
     */
    function getMetadata({ template: Struct }): { title: string, author: string, subject: string, keywords: string, description: string };

    /**
     * Scan document body, headers, footers for all {{...}} placeholder strings
     * @node docx_list_placeholders @alias docxListPlaceholders
     * @param template — DOCX template file
     * @returns placeholders — List of placeholder strings found
     * @impure has side effects / drives control flow
     */
    function listPlaceholders({ template: Struct }): string[];

    /**
     * Concatenate multiple DOCX documents into one, with optional page breaks between them
     * @node docx_merge @alias docxMerge
     * @param documents — Array of DOCX file paths to merge in order
     * @param pageBreak (optional) — Insert page break between documents
     * @param output — Where to save the merged file
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function merge({ documents: Struct[], pageBreak?: bool, output: Struct }): Struct;

    /**
     * Remove paragraphs containing a specific placeholder. Useful for conditional content.
     * @node docx_remove_paragraph @alias docxRemoveParagraph
     * @param template — DOCX file to modify
     * @param placeholder — Text to search for — paragraphs containing this are removed
     * @param output — Where to save the result
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function removeParagraph({ template: Struct, placeholder: string, output: Struct }): Struct;

    /**
     * Replace an image in a DOCX file by matching alt text or shape name
     * @node docx_replace_image @alias docxReplaceImage
     * @param template — DOCX template file
     * @param identifier — Alt text or shape name of the image to replace
     * @param image — Replacement image file
     * @param scaleMode (optional) — How to handle dimensions: KeepWidth (proportional), KeepHeight (proportional), Stretch (force both, may distort), or None (use new image size)
     * @param output — Where to save the resulting DOCX file
     * @returns result — Path to the output file
     * @impure has side effects / drives control flow
     */
    function replaceImage({ template: Struct, identifier: string, image: Struct, scaleMode?: string, output: Struct }): Struct;

    /**
     * Find a table containing a placeholder, duplicate that row for each data item, replacing placeholders per row
     * @node docx_replace_table_row @alias docxReplaceTableRow
     * @param template — DOCX template file
     * @param placeholder — Placeholder in the template row (e.g. {{item}})
     * @param data — JSON array of objects — each object's keys match placeholders in the row
     * @param output — Where to save the result
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function replaceTableRow({ template: Struct, placeholder: string, data: string, output: Struct }): Struct;

    /**
     * Replace text placeholders in a DOCX template file with plain text or markdown
     * @node docx_replace_text @alias docxReplaceText
     * @param template — DOCX template file
     * @param placeholder — Placeholder text to find (e.g. {{name}})
     * @param replacement — Replacement text (supports markdown when enabled)
     * @param useMarkdown (optional) — Parse the replacement text as markdown
     * @param output — Where to save the resulting DOCX file
     * @returns result — Path to the output file
     * @impure has side effects / drives control flow
     */
    function replaceText({ template: Struct, placeholder: string, replacement: string, useMarkdown?: bool, output: Struct }): Struct;

    /**
     * Set title, author, subject, keywords, description in document metadata
     * @node docx_set_metadata @alias docxSetMetadata
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
    function setMetadata({ template: Struct, title?: string, author?: string, subject?: string, keywords?: string, description?: string, output: Struct }): Struct;
}

declare namespace image {
    // === Data/QR ===

    /**
     * Encode text as a barcode image
     * @node write_qrcode @alias writeQrcode
     * @param data — Text to encode
     * @param format (optional) — Barcode Format
     * @param scale (optional) — Pixels per barcode module
     * @param margin (optional) — Quiet zone in modules
     * @returns imageOut — Barcode image
     * @impure has side effects / drives control flow
     */
    function writeBarcode({ data: string, format?: string, scale?: int, margin?: int }): Struct;

    // === Image ===

    /**
     * Decode a still image and write it as PNG, JPEG, GIF, WebP, or AVIF
     * @node video_convert_image_format @alias videoConvertImageFormat
     * @param source — Source image FlowPath
     * @param target — Target image FlowPath
     * @param format (optional) — Output image format, or auto from target extension
     * @returns result — Written image FlowPath
     * @returns report — Image conversion report
     * @impure has side effects / drives control flow
     */
    function convertFormat({ source: Struct, target: Struct, format?: string }): { result: Struct, report: Struct };

    /**
     * Apply crop, resize, flip, rotate, blur, and color filters to a still image
     * @node video_transform_image @alias videoTransformImage
     * @param source — Source image FlowPath
     * @param target — Target image FlowPath
     * @param format (optional) — Output image format, or auto from target extension
     * @param cropX (optional)
     * @param cropY (optional)
     * @param cropWidth (optional)
     * @param cropHeight (optional)
     * @param resizeWidth (optional)
     * @param resizeHeight (optional)
     * @param rotateDegrees (optional)
     * @param blurRadius (optional)
     * @param flipHorizontal (optional)
     * @param flipVertical (optional)
     * @param brightness (optional) — -1.0 to 1.0
     * @param contrast (optional) — 1.0 keeps contrast unchanged
     * @param saturation (optional) — 1.0 keeps saturation unchanged
     * @returns result — Written image FlowPath
     * @returns report — Image transform report
     * @impure has side effects / drives control flow
     */
    function transform({ source: Struct, target: Struct, format?: string, cropX?: int, cropY?: int, cropWidth?: int, cropHeight?: int, resizeWidth?: int, resizeHeight?: int, rotateDegrees?: int, blurRadius?: int, flipHorizontal?: bool, flipVertical?: bool, brightness?: float, contrast?: float, saturation?: float }): { result: Struct, report: Struct };

    // === Image/Annotate ===

    /**
     * Draw Bounding Boxes
     * @node draw_boxes @receiver image_in @alias drawBoxes
     * @param imageIn — Image object (receiver: `this` in `x.drawBoxes(...)`)
     * @param bboxes — Bounding Boxes
     * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
     * @returns imageOut — Image with Bounding Boxes
     * @impure has side effects / drives control flow
     */
    function drawBoxes(this: NodeImage, { imageIn: Struct, bboxes: Struct[], useRef?: bool }): Struct;

    /**
     * Make Bounding Box
     * @node make_boxe @alias makeBoxe
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
    function makeBox({ definition?: string, classIdx?: int, score?: float, x1: float, y1: float, x2: float, y2: float }): Struct;

    // === Image/Content ===

    /**
     * Read image from path
     * @node read_image @alias readImage
     * @param path — FlowPath
     * @param applyExif (optional) — Apply Exif Orientation
     * @returns imageOut — Image object
     * @impure has side effects / drives control flow
     */
    function read({ path: Struct, applyExif?: bool }): Struct;

    /**
     * Read/Decode QR Codes and Barcodes
     * @node read_barcodes @receiver image_in @alias readBarcodes
     * @param imageIn — Image object (receiver: `this` in `x.readBarcodes(...)`)
     * @param options (optional) — Barcode decoding options
     * @returns results — Detected/Decoded Codes
     * @impure has side effects / drives control flow
     */
    function readBarcodes(this: NodeImage, { imageIn: Struct, options?: Struct }): Struct[];

    /**
     * Read image from path
     * @node read_image_url @alias readImageUrl
     * @param signedUrl — Signed Url
     * @param applyExif (optional) — Apply Exif Orientation
     * @returns imageOut — Image object
     * @impure has side effects / drives control flow
     */
    function readUrl({ signedUrl: string, applyExif?: bool }): Struct;

    /**
     * Write image to path
     * @node write_image @receiver image_in @alias writeImage
     * @param imageIn — The image to write to path (receiver: `this` in `x.write(...)`)
     * @param path — FlowPath
     * @param type (optional) — Image Type
     * @param quality (optional) — Encoding Quality
     * @impure has side effects / drives control flow
     */
    function write(this: NodeImage, { imageIn: Struct, path: Struct, type?: string, quality?: int }): void;

    // === Image/Metadata ===

    /**
     * Get Image Dimensions
     * @node get_dimensions @receiver image_in @alias getDimensions
     * @param imageIn — Image object (receiver: `this` in `x.getDimensions(...)`)
     * @returns width — Image Width
     * @returns height — Image Height
     * @impure has side effects / drives control flow
     */
    function getDimensions(this: NodeImage, { imageIn: Struct }): { width: int, height: int };

    // === Image/Overlay ===

    /**
     * Overlay one image on top of another with configurable position, size, opacity and fit mode
     * @node image_overlay @receiver base_image @alias imageOverlay
     * @param baseImage — The background image (receiver: `this` in `x.overlay(...)`)
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
    function overlay(this: NodeImage, { baseImage: Struct, overlayImage: Struct, useRef?: bool, x?: int, y?: int, maxW?: int, maxH?: int, opacity?: float, fitMode?: string }): Struct;

    /**
     * Draw text on top of an image with configurable font size, position, and color
     * @node text_overlay @receiver base_image @alias textOverlay
     * @param baseImage — The background image to draw text on (receiver: `this` in `x.textOverlay(...)`)
     * @param text (optional) — The text string to render
     * @param useRef (optional) — Use reference of the base image, transforming the original instead of a copy
     * @param x (optional) — Horizontal offset in pixels from the left edge
     * @param y (optional) — Vertical offset in pixels from the top edge
     * @param fontSize (optional) — Font size in pixels
     * @param color (optional) — Text color as hex string (e.g. #FF0000 or #FF0000AA for alpha)
     * @returns imageOut — Result image with text rendered
     * @impure has side effects / drives control flow
     */
    function textOverlay(this: NodeImage, { baseImage: Struct, text?: string, useRef?: bool, x?: int, y?: int, fontSize?: float, color?: string }): Struct;

    // === Image/Transform ===

    /**
     * Adjust Image Contrast
     * @node contrast_image @receiver image_in @alias contrastImage
     * @param imageIn — Image object (receiver: `this` in `x.contrast(...)`)
     * @param contrast — Contrast
     * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
     * @returns imageOut — Image with Applied Contrast
     * @impure has side effects / drives control flow
     */
    function contrast(this: NodeImage, { imageIn: Struct, contrast: float, useRef?: bool }): Struct;

    /**
     * Convert Image Color/Pixel Type (e.g. to Grayscale)
     * @node convert_image @receiver image_in @alias convertImage
     * @param imageIn — Image object (receiver: `this` in `x.convertColor(...)`)
     * @param pixelType (optional) — Target Pixel Type
     * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
     * @returns imageOut — Image with Target Color/Pixel Type
     * @impure has side effects / drives control flow
     */
    function convertColor(this: NodeImage, { imageIn: Struct, pixelType?: string, useRef?: bool }): Struct;

    /**
     * Crop Image
     * @node crop_image @receiver image_in @alias cropImage
     * @param imageIn — Image object (receiver: `this` in `x.crop(...)`)
     * @param bbox — Bounding Box
     * @param useRef (optional) — Use Reference of the image, transforming the original instead of a copy
     * @returns imageOut — Cropped Image object
     * @impure has side effects / drives control flow
     */
    function crop(this: NodeImage, { imageIn: Struct, bbox: Struct, useRef?: bool }): Struct;

    /**
     * Resize Image
     * @node resize_image @receiver image_in @alias resizeImage
     * @param imageIn — Image object (receiver: `this` in `x.resize(...)`)
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
    function resize(this: NodeImage, { imageIn: Struct, useRef?: bool, mode?: string, filter?: string, widthIn?: int, heightIn?: int }): { imageOut: Struct, widthOut: int, heightOut: int };

    // === Web/Camera ===

    /**
     * Writes an image to a data URL
     * @node image_write_dataurl @receiver image @alias imageWriteDataurl
     * @param image — The image to write to a data URL (receiver: `this` in `x.toDataUrl(...)`)
     * @param format (optional) — The format of the image (e.g., png, jpeg)
     * @returns url — The data URL of the written image
     * @impure has side effects / drives control flow
     */
    function toDataUrl(this: NodeImage, { image: Struct, format?: string }): string;
}

declare namespace pdf {
    // === Document/PDF ===

    /**
     * Stamp an image at a specified position on selected PDF pages.
     * @node pdf_add_image_stamp @alias pdfAddImageStamp
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
    function addImageStamp({ template: Struct, image: Struct, x?: float, y?: float, width?: float, height?: float, pages?: int[], output: Struct }): Struct;

    /**
     * Add 'Page X of Y' labels to each page of a PDF.
     * @node pdf_add_page_numbers @alias pdfAddPageNumbers
     * @param template — PDF file
     * @param position (optional) — Position: bottom-center, bottom-right, bottom-left
     * @param fontSize (optional) — Font size in points
     * @param margin (optional) — Margin from edge in points
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function addPageNumbers({ template: Struct, position?: string, fontSize?: float, margin?: float, output: Struct }): Struct;

    /**
     * Overlay a diagonal text watermark on all pages. Default: #FF4343 at 15% opacity.
     * @node pdf_add_watermark @alias pdfAddWatermark
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
    function addWatermark({ template: Struct, text: string, fontSize?: float, color?: string, opacity?: float, rotationDeg?: float, output: Struct }): Struct;

    /**
     * Optimize and compress a PDF by deduplicating objects and compressing streams.
     * @node pdf_compress @alias pdfCompress
     * @param template — PDF file
     * @param output — Save path
     * @returns result — Output file path
     * @returns originalSize — Size in bytes before compression
     * @returns compressedSize — Size in bytes after compression
     * @impure has side effects / drives control flow
     */
    function compress({ template: Struct, output: Struct }): { result: Struct, originalSize: int, compressedSize: int };

    /**
     * Remove password protection from a PDF using the owner or user password.
     * @node pdf_decrypt @alias pdfDecrypt
     * @param template — Encrypted PDF file
     * @param password — Owner or user password
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function decrypt({ template: Struct, password: string, output: Struct }): Struct;

    /**
     * Encrypt a PDF with a user password for restricted access.
     * @node pdf_encrypt @alias pdfEncrypt
     * @param template — PDF file
     * @param userPassword — Password required to open
     * @param ownerPassword (optional) — Password for full access (optional, defaults to user password)
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function encrypt({ template: Struct, userPassword: string, ownerPassword?: string, output: Struct }): Struct;

    /**
     * Extract specific pages (non-contiguous) from a PDF
     * @node pdf_extract_pages @alias pdfExtractPages
     * @param template — PDF file
     * @param pages — Array of page numbers to extract (1-based)
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function extractPages({ template: Struct, pages: int[], output: Struct }): Struct;

    /**
     * Extract all text content from a PDF document.
     * @node pdf_extract_text @alias pdfExtractText
     * @param template — PDF file
     * @returns text — Extracted text
     * @impure has side effects / drives control flow
     */
    function extractText({ template: Struct }): string;

    /**
     * Sets the value of a named AcroForm field in a PDF document.
     * @node pdf_fill_form @alias pdfFillForm
     * @param template — PDF file containing form fields
     * @param fieldName — Name of the AcroForm field to fill
     * @param fieldValue — Value to set on the form field
     * @param output — Path to save the filled PDF
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function fillForm({ template: Struct, fieldName: string, fieldValue: string, output: Struct }): Struct;

    /**
     * Typesets Markdown into a paginated PDF with selectable text, tables, code blocks, charts and embedded images
     * @node pdf_create_from_markdown @alias pdfCreateFromMarkdown
     * @param markdown — Markdown source to typeset
     * @param output — Save path
     * @param pageSize (optional) — Page geometry
     * @param embedImages (optional) — Download and embed images referenced by the Markdown. Disable to render placeholders instead.
     * @param pageNumbers (optional) — Print a page number in the footer
     * @param title (optional) — Document title. Also sets the running header and the cover block.
     * @param subtitle (optional) — Secondary line under the title on the cover block
     * @param cover (optional) — Open the document with the accent title block
     * @param author (optional) — Document author metadata
     * @returns result — Output file path
     * @returns pages — Number of pages written
     * @impure has side effects / drives control flow
     */
    function fromMarkdown({ markdown: string, output: Struct, pageSize?: string, embedImages?: bool, pageNumbers?: bool, title?: string, subtitle?: string, cover?: bool, author?: string }): { result: Struct, pages: int };

    /**
     * Read title, author, subject, keywords, and page count from a PDF.
     * @node pdf_get_metadata @alias pdfGetMetadata
     * @param template — PDF file
     * @returns title — Document title
     * @returns author — Author
     * @returns subject — Subject
     * @returns keywords — Keywords
     * @returns pageCount — Number of pages
     * @impure has side effects / drives control flow
     */
    function getMetadata({ template: Struct }): { title: string, author: string, subject: string, keywords: string, pageCount: int };

    /**
     * Reads a PDF and returns all AcroForm field names so you know which fields are available to fill.
     * @node pdf_list_form_fields @alias pdfListFormFields
     * @param template — PDF file containing form fields
     * @returns fieldNames — Array of all form field names in the PDF
     * @returns fieldCount — Total number of form fields
     * @impure has side effects / drives control flow
     */
    function listFormFields({ template: Struct }): { fieldNames: string[], fieldCount: int };

    /**
     * Concatenate multiple PDF files into one
     * @node pdf_merge @alias pdfMerge
     * @param documents — Array of PDF file paths to merge in order
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function merge({ documents: Struct[], output: Struct }): Struct;

    /**
     * Return the number of pages in a PDF file
     * @node pdf_page_count @alias pdfPageCount
     * @param template — PDF file
     * @returns count — Number of pages
     * @impure has side effects / drives control flow
     */
    function pageCount({ template: Struct }): int;

    /**
     * Replaces an image XObject in a PDF by name. Any image format is accepted and automatically converted to JPEG.
     * @node pdf_replace_image @alias pdfReplaceImage
     * @param template — PDF file containing the image to replace
     * @param imageName — XObject image name (e.g. "Im0", "Image1")
     * @param image — Replacement image file (any format — auto-converted to JPEG)
     * @param scaleMode (optional) — How to handle dimensions: KeepWidth (proportional), KeepHeight (proportional), Stretch (force both, may distort), or None (use new image size)
     * @param output — Path to save the modified PDF
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function replaceImage({ template: Struct, imageName: string, image: Struct, scaleMode?: string, output: Struct }): Struct;

    /**
     * Attempts to find and replace text in a PDF. Best-effort: PDF text replacement may not work for all documents due to complex text encoding and fragmented content streams.
     * @node pdf_replace_text @alias pdfReplaceText
     * @param template — PDF file to modify
     * @param placeholder — Text to find in the PDF
     * @param replacement — Plain text replacement value
     * @param output — Path to save the modified PDF
     * @returns result — Output file path
     * @returns replacedCount — Number of text replacements made
     * @impure has side effects / drives control flow
     */
    function replaceText({ template: Struct, placeholder: string, replacement: string, output: Struct }): { result: Struct, replacedCount: int };

    /**
     * Rotate pages by 90, 180, or 270 degrees
     * @node pdf_rotate_pages @alias pdfRotatePages
     * @param template — PDF file
     * @param pages (optional) — Page numbers to rotate (1-based). Empty array = all pages.
     * @param rotation (optional) — Rotation degrees: 90, 180, or 270
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function rotatePages({ template: Struct, pages?: int[], rotation?: int, output: Struct }): Struct;

    /**
     * Set title, author, subject, and keywords in a PDF's Info dictionary.
     * @node pdf_set_metadata @alias pdfSetMetadata
     * @param template — PDF file
     * @param title (optional) — Document title
     * @param author (optional) — Author
     * @param subject (optional) — Subject
     * @param keywords (optional) — Keywords
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function setMetadata({ template: Struct, title?: string, author?: string, subject?: string, keywords?: string, output: Struct }): Struct;

    /**
     * Extract a page range from a PDF into a new file
     * @node pdf_split @alias pdfSplit
     * @param template — PDF file
     * @param startPage (optional) — First page to extract (1-based)
     * @param endPage (optional) — Last page to extract (1-based, inclusive)
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function split({ template: Struct, startPage?: int, endPage?: int, output: Struct }): Struct;

    // === Image/PDF ===

    /**
     * Count pages in a PDF
     * @node pdf_page_count @alias pdfPageCount
     * @param pdf — PDF file
     * @returns pageCount — Page count
     * @impure has side effects / drives control flow
     */
    function pageCount({ pdf: Struct }): int;

    /**
     * Render a single PDF page as an image
     * @node pdf_page_to_image @alias pdfPageToImage
     * @param pdf — PDF file
     * @param page (optional) — 1-based page number
     * @param scale (optional) — Render scale
     * @param bgColor (optional) — Background color for the rendered page
     * @returns image — Rendered image
     * @impure has side effects / drives control flow
     */
    function pageToImage({ pdf: Struct, page?: int, scale?: float, bgColor?: string }): Struct;

    /**
     * Render every PDF page as an ordered image array
     * @node pdf_to_images @alias pdfToImages
     * @param pdf — PDF file
     * @param scale (optional) — Render scale
     * @param bgColor (optional) — Background color for rendered pages
     * @returns images — Rendered images
     * @impure has side effects / drives control flow
     */
    function toImages({ pdf: Struct, scale?: float, bgColor?: string }): Struct[];
}

declare namespace pptx {
    // === Document/PPTX ===

    /**
     * Embed a simple bar chart on a PPTX slide using DrawingML chart XML.
     * @node pptx_add_chart @alias pptxAddChart
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
    function addChart({ template: Struct, slideNumber?: int, chartType?: string, categories: string[], values: float[], seriesName?: string, x?: float, y?: float, width?: float, height?: float, output: Struct }): Struct;

    /**
     * Place an image at a specified position on a PPTX slide.
     * @node pptx_add_image_to_slide @alias pptxAddImageToSlide
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
    function addImage({ template: Struct, image: Struct, slideNumber?: int, x?: float, y?: float, width?: float, height?: float, output: Struct }): Struct;

    /**
     * Set or replace speaker notes for a slide
     * @node pptx_add_notes @alias pptxAddNotes
     * @param template — Path to the PPTX file
     * @param slideIndex (optional) — Which slide to set notes for (1-based)
     * @param notes — Speaker notes text
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function addNotes({ template: Struct, slideIndex?: int, notes: string, output: Struct }): Struct;

    /**
     * Add a shape (rectangle, ellipse, arrow, etc.) to a PPTX slide.
     * @node pptx_add_shape @alias pptxAddShape
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
    function addShape({ template: Struct, slideNumber?: int, shape?: string, x?: float, y?: float, width?: float, height?: float, fillColor?: string, lineColor?: string, text?: string, output: Struct }): Struct;

    /**
     * Add a blank slide to a PPTX presentation.
     * @node pptx_add_slide @alias pptxAddSlide
     * @param template — PPTX file
     * @param output — Save path
     * @returns result — Output file path
     * @returns slideNumber — New slide's index (1-based)
     * @impure has side effects / drives control flow
     */
    function addSlide({ template: Struct, output: Struct }): { result: Struct, slideNumber: int };

    /**
     * Add a branded table to a PPTX slide. Header row uses #FF4343 with white text.
     * @node pptx_add_table_to_slide @alias pptxAddTableToSlide
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
    function addTable({ template: Struct, slideNumber?: int, headers: string[], rows: string[], x?: float, y?: float, width?: float, rowHeight?: float, output: Struct }): Struct;

    /**
     * Add a styled text box to a specific slide in a PPTX.
     * @node pptx_add_text_box @alias pptxAddTextBox
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
    function addTextBox({ template: Struct, slideNumber?: int, text: string, x?: float, y?: float, width?: float, height?: float, fontSize?: float, fontColor?: string, bold?: bool, output: Struct }): Struct;

    /**
     * Create a blank PPTX presentation with Flow Like brand theme (16:9, Calibri, #FF4343 accent).
     * @node pptx_create @alias pptxCreate
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function create({ output: Struct }): Struct;

    /**
     * Remove a slide at the given index from a PPTX file
     * @node pptx_delete_slide @alias pptxDeleteSlide
     * @param template — Path to the PPTX file
     * @param slideIndex (optional) — Index of the slide to delete (1-based)
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function deleteSlide({ template: Struct, slideIndex?: int, output: Struct }): Struct;

    /**
     * Clone a slide at a given index, inserting the copy at a target position. Preserves formatting, layouts, and master references.
     * @node pptx_duplicate_slide @alias pptxDuplicateSlide
     * @param template — Path to the PPTX file
     * @param slideIndex (optional) — Index of the slide to clone (1-based)
     * @param targetIndex (optional) — Position to insert the cloned slide (1-based)
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function duplicateSlide({ template: Struct, slideIndex?: int, targetIndex?: int, output: Struct }): Struct;

    /**
     * Extract all text content from all slides as plain text
     * @node pptx_extract_text @alias pptxExtractText
     * @param template — Path to the PPTX file
     * @returns text — Extracted text from all slides
     * @impure has side effects / drives control flow
     */
    function extractText({ template: Struct }): string;

    /**
     * Read presentation metadata (title, author, subject, keywords)
     * @node pptx_get_metadata @alias pptxGetMetadata
     * @param template — PPTX file
     * @returns title — Document title
     * @returns author — Document author
     * @returns subject — Document subject
     * @returns keywords — Document keywords
     * @impure has side effects / drives control flow
     */
    function getMetadata({ template: Struct }): { title: string, author: string, subject: string, keywords: string };

    /**
     * Scan all slides for {{...}} placeholder strings
     * @node pptx_list_placeholders @alias pptxListPlaceholders
     * @param template — Path to the PPTX file
     * @returns placeholders — List of unique placeholder names found
     * @impure has side effects / drives control flow
     */
    function listPlaceholders({ template: Struct }): string[];

    /**
     * Combine slides from multiple PPTX files into one. The base file's theme and masters are preserved.
     * @node pptx_merge @alias pptxMerge
     * @param base — Base PPTX file (theme/masters kept)
     * @param additional — Additional PPTX files to merge (array of paths)
     * @param output — Where to save the merged file
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function merge({ base: Struct, additional: Struct[], output: Struct }): Struct;

    /**
     * Move a slide from one position to another
     * @node pptx_reorder_slides @alias pptxReorderSlides
     * @param template — Path to the PPTX file
     * @param fromIndex (optional) — Current position of the slide (1-based)
     * @param toIndex (optional) — Target position for the slide (1-based)
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function reorderSlides({ template: Struct, fromIndex?: int, toIndex?: int, output: Struct }): Struct;

    /**
     * Replaces images in a PowerPoint (PPTX) file by matching alt text or shape name
     * @node pptx_replace_image @alias pptxReplaceImage
     * @param template — Path to the PPTX template file
     * @param identifier — Alt text or shape name of the image to replace
     * @param image — Path to the replacement image file
     * @param scaleMode (optional) — How to handle dimensions: KeepWidth (proportional), KeepHeight (proportional), Stretch (force both, may distort), or None (use new image size)
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function replaceImage({ template: Struct, identifier: string, image: Struct, scaleMode?: string, output: Struct }): Struct;

    /**
     * Populate a table on a slide that contains a placeholder in its first cell with structured data (JSON array of arrays). Inherits the table's existing styling.
     * @node pptx_replace_table_data @alias pptxReplaceTableData
     * @param template — Path to the PPTX file
     * @param slideIndex (optional) — Which slide contains the table (1-based)
     * @param placeholder — Placeholder text to find in the table
     * @param data — JSON array of arrays with table data
     * @param hasHeader (optional) — Whether the first row of data is a header row
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function replaceTableData({ template: Struct, slideIndex?: int, placeholder: string, data: string, hasHeader?: bool, output: Struct }): Struct;

    /**
     * Replaces text placeholders in a PowerPoint (PPTX) file with plain or markdown-formatted text
     * @node pptx_replace_text @alias pptxReplaceText
     * @param template — Path to the PPTX template file
     * @param placeholder — The placeholder text to find in the template
     * @param replacement — The replacement text (supports markdown when enabled)
     * @param useMarkdown (optional) — Parse replacement text as markdown for rich formatting
     * @param output — Path where the resulting PPTX file will be saved
     * @returns result — Path to the generated PPTX file
     * @impure has side effects / drives control flow
     */
    function replaceText({ template: Struct, placeholder: string, replacement: string, useMarkdown?: bool, output: Struct }): Struct;

    /**
     * Set title, author, subject, keywords in presentation metadata
     * @node pptx_set_metadata @alias pptxSetMetadata
     * @param template — PPTX file
     * @param title (optional) — Document title
     * @param author (optional) — Document author
     * @param subject (optional) — Document subject
     * @param keywords (optional) — Comma-separated keywords
     * @param output — Save path
     * @returns result — Output file path
     * @impure has side effects / drives control flow
     */
    function setMetadata({ template: Struct, title?: string, author?: string, subject?: string, keywords?: string, output: Struct }): Struct;

    /**
     * Return the number of slides in a PPTX file
     * @node pptx_slide_count @alias pptxSlideCount
     * @param template — Path to the PPTX file
     * @returns count — Number of slides in the presentation
     * @impure has side effects / drives control flow
     */
    function slideCount({ template: Struct }): int;
}

declare namespace video {
    // === Diagnostics ===

    /**
     * Choose the preferred compiled backend for a codec and operation
     * @node video_pick_codec_backend @alias videoPickCodecBackend
     * @param codec (optional) — Codec id such as h264, h265, av1, aac, or mp3
     * @param direction (optional) — decode or encode
     * @returns selection — Preferred backend selection
     * @returns support — Compiled codec support registry
     * @impure has side effects / drives control flow
     */
    function pickCodecBackend({ codec?: string, direction?: string }): { selection: Struct, support: Struct[] };

    /**
     * Report compiled video-utils-rs features and recommended codec backend lanes
     * @node video_probe_codec_backends @alias videoProbeCodecBackends
     * @returns backends — Recommended codec backends
     * @returns features — Compiled video-utils-rs feature set
     * @impure has side effects / drives control flow
     */
    function probeCodecBackends(): { backends: Struct[], features: Struct };

    /**
     * Check whether the current host can decode or encode a codec through native platform APIs
     * @node video_probe_platform_codec @alias videoProbePlatformCodec
     * @param codec (optional) — Codec id such as h264, h265, av1, aac, or mp3
     * @param direction (optional) — decode or encode
     * @returns probe — Platform codec probe result
     * @impure has side effects / drives control flow
     */
    function probePlatformCodec({ codec?: string, direction?: string }): Struct;

    // === Streaming ===

    /**
     * Write an HLS media playlist plus MPEG-TS or fMP4 segments
     * @node video_package_hls_vod @alias videoPackageHlsVod
     * @param source — Source media FlowPath
     * @param playlist — Target .m3u8 FlowPath
     * @param targetDurationSeconds (optional) — Target segment duration in seconds
     * @param segmentFormat (optional) — Segment container format
     * @param segmentTrackId (optional) — Track used for segment boundaries; 0 chooses first video/audio
     * @param copyAllTracks (optional) — Include every stream in each segment
     * @param segmentPrefix (optional) — Optional segment object-key prefix
     * @param initSegmentName (optional) — Optional fMP4 init segment name
     * @param uriPrefix (optional) — Optional URI prefix written into playlist
     * @returns playlistOut — Written playlist FlowPath
     * @returns segments — Written segment FlowPaths
     * @returns report — Written HLS package details
     * @impure has side effects / drives control flow
     */
    function packageHlsVod({ source: Struct, playlist: Struct, targetDurationSeconds?: float, segmentFormat?: string, segmentTrackId?: int, copyAllTracks?: bool, segmentPrefix?: string, initSegmentName?: string, uriPrefix?: string }): { playlistOut: Struct, segments: Struct[], report: Struct };

    // === Subtitles ===

    /**
     * Mux an SRT or WebVTT sidecar into a Matroska subtitle track
     * @node video_add_subtitle_track @alias videoAddSubtitleTrack
     * @param source — Source media FlowPath
     * @param sidecar — Subtitle sidecar FlowPath
     * @param target — Target Matroska FlowPath
     * @param format (optional) — Subtitle sidecar format
     * @param trackId (optional) — Subtitle track id to create
     * @param language (optional) — Optional language tag
     * @returns result — Written media FlowPath
     * @returns report — Subtitle mux report
     * @impure has side effects / drives control flow
     */
    function addSubtitleTrack({ source: Struct, sidecar: Struct, target: Struct, format?: string, trackId?: int, language?: string }): { result: Struct, report: Struct };

    /**
     * Render an SRT/WebVTT sidecar into video frames and mux the result
     * @node video_burn_subtitles @alias videoBurnSubtitles
     * @param source — Source media FlowPath
     * @param sidecar — Subtitle sidecar FlowPath
     * @param target — Target media FlowPath
     * @param format (optional) — srt or webvtt
     * @param outputCodec (optional) — Video codec to encode
     * @param videoTrackId (optional) — Video track id, or 0 for default
     * @param preserveNonVideo (optional) — Copy non-video packets when possible
     * @param bitrate (optional) — Target bitrate in bits per second, or 0 for backend default
     * @param scale (optional) — Subtitle render scale
     * @param marginBottom (optional) — Subtitle bottom margin in pixels
     * @returns result — Written media FlowPath
     * @returns report — Subtitle burn-in report
     * @impure has side effects / drives control flow
     */
    function burnSubtitles({ source: Struct, sidecar: Struct, target: Struct, format?: string, outputCodec?: string, videoTrackId?: int, preserveNonVideo?: bool, bitrate?: int, scale?: int, marginBottom?: int }): { result: Struct, report: Struct };

    /**
     * Extract a subtitle track to an SRT or WebVTT sidecar
     * @node video_extract_subtitle_track @alias videoExtractSubtitleTrack
     * @param source — Source media FlowPath
     * @param target — Target sidecar FlowPath
     * @param format (optional) — Output subtitle format
     * @param trackId (optional) — Subtitle track id; 0 uses first subtitle track
     * @returns result — Written sidecar FlowPath
     * @returns report — Subtitle extraction report
     * @impure has side effects / drives control flow
     */
    function extractSubtitleTrack({ source: Struct, target: Struct, format?: string, trackId?: int }): { result: Struct, report: Struct };

    /**
     * Parse SRT or WebVTT sidecar subtitles into cue structs
     * @node video_parse_subtitles @alias videoParseSubtitles
     * @param sidecar — Subtitle sidecar FlowPath
     * @param format (optional) — Subtitle format
     * @returns cues — Parsed subtitle cues
     * @returns count — Cue count
     * @impure has side effects / drives control flow
     */
    function parseSubtitles({ sidecar: Struct, format?: string }): { cues: Struct[], count: int };

    /**
     * Offset all SRT or WebVTT cues and write a new sidecar
     * @node video_shift_subtitle_file @alias videoShiftSubtitleFile
     * @param source — Subtitle sidecar FlowPath
     * @param target — Target sidecar FlowPath
     * @param format (optional) — Subtitle format
     * @param offsetMs (optional) — Positive or negative subtitle offset in milliseconds
     * @returns result — Written sidecar FlowPath
     * @returns count — Shifted cue count
     * @impure has side effects / drives control flow
     */
    function shiftSubtitleFile({ source: Struct, target: Struct, format?: string, offsetMs?: int }): { result: Struct, count: int };

    /**
     * Write subtitle cue structs to an SRT or WebVTT sidecar
     * @node video_write_subtitles @alias videoWriteSubtitles
     * @param cues — Subtitle cues
     * @param target — Subtitle sidecar FlowPath
     * @param format (optional) — Subtitle format
     * @returns result — Written sidecar FlowPath
     * @returns count — Cue count
     * @impure has side effects / drives control flow
     */
    function writeSubtitles({ cues: Struct[], target: Struct, format?: string }): { result: Struct, count: int };

    // === Video/Containers ===

    /**
     * Rewrap compatible streams into another container without decoding
     * @node video_remux @alias videoRemux
     * @param source — Source media FlowPath
     * @param target — Target media FlowPath
     * @returns result — Written media FlowPath
     * @returns report — Remux operation report
     * @impure has side effects / drives control flow
     */
    function remux({ source: Struct, target: Struct }): { result: Struct, report: Struct };

    // === Video/Editing ===

    /**
     * Concatenate packet-copy-compatible media files
     * @node video_concat @alias videoConcat
     * @param sources — Media FlowPaths in concatenation order
     * @param target — Target media FlowPath
     * @returns result — Written media FlowPath
     * @returns packetCount — Packets written
     * @impure has side effects / drives control flow
     */
    function concat({ sources: Struct[], target: Struct }): { result: Struct, packetCount: int };

    /**
     * Trim a media file using a keyframe-aligned packet range
     * @node video_trim_keyframes @alias videoTrimKeyframes
     * @param source — Source media FlowPath
     * @param target — Target media FlowPath
     * @param startSeconds (optional) — Requested start time
     * @param endSeconds (optional) — Requested end time
     * @param trackId (optional) — Boundary video track; 0 uses first video track
     * @returns result — Written media FlowPath
     * @returns packetCount — Packets written
     * @returns boundaryTrackId — Track used for keyframe selection
     * @impure has side effects / drives control flow
     */
    function trimKeyframes({ source: Struct, target: Struct, startSeconds?: float, endSeconds?: float, trackId?: int }): { result: Struct, packetCount: int, boundaryTrackId: int };

    // === Video/Inspect ===

    /**
     * Detect the media container for a FlowPath object
     * @node video_detect_container @alias videoDetectContainer
     * @param source — Media FlowPath to inspect
     * @returns container — Detected media container
     * @impure has side effects / drives control flow
     */
    function detectContainer({ source: Struct }): Struct;

    /**
     * Extract stream metadata from a media FlowPath
     * @node video_probe_media_info @alias videoProbeMediaInfo
     * @param source — Media FlowPath to inspect
     * @returns media — Container and stream metadata
     * @returns streams — Detected media streams
     * @impure has side effects / drives control flow
     */
    function probeMediaInfo({ source: Struct }): { media: Struct, streams: Struct[] };

    // === Video/Packets ===

    /**
     * Convert H.264/H.265/AAC packet bitstream framing into an elementary output file
     * @node video_bitstream_convert @alias videoBitstreamConvert
     * @param source — Source media FlowPath
     * @param target — Target elementary FlowPath
     * @param conversion (optional) — h264_annex_b, h264_length_prefixed, h265_annex_b, h265_length_prefixed, aac_adts, or aac_raw
     * @param trackId (optional) — Track id, or 0 to select by conversion codec
     * @returns result — Written elementary FlowPath
     * @returns report — Bitstream conversion report
     * @impure has side effects / drives control flow
     */
    function bitstreamConvert({ source: Struct, target: Struct, conversion?: string, trackId?: int }): { result: Struct, report: Struct };

    /**
     * Rebase packet timestamps so each track starts at zero or later
     * @node video_normalize_timestamps @alias videoNormalizeTimestamps
     * @param source — Source media FlowPath
     * @param target — Target media FlowPath
     * @returns result — Written media FlowPath
     * @returns packetCount — Packets written
     * @impure has side effects / drives control flow
     */
    function normalizeTimestamps({ source: Struct, target: Struct }): { result: Struct, packetCount: int };

    // === Video/Planning ===

    /**
     * Check whether source streams can be packet-copied into a target container
     * @node video_check_remux_compatibility @alias videoCheckRemuxCompatibility
     * @param source — Source media FlowPath
     * @param target — Target FlowPath with desired extension
     * @returns report — Detailed remux compatibility report
     * @impure has side effects / drives control flow
     */
    function checkRemuxCompatibility({ source: Struct, target: Struct }): Struct;

    // === Video/Preview ===

    /**
     * Sample decoded frames and write a preview grid image
     * @node video_contact_sheet @alias videoContactSheet
     * @param source — Source media FlowPath
     * @param target — Target image FlowPath
     * @param maxFrames (optional) — Maximum frames in the sheet
     * @param everyNFrames (optional) — Sampling interval in decoded frames
     * @param columns (optional) — Grid column count
     * @param cellWidth (optional) — Cell width in pixels
     * @param cellHeight (optional) — Cell height in pixels
     * @param videoTrackId (optional) — Video track id, or 0 for default
     * @param format (optional) — Output image format, or auto from target extension
     * @returns result — Written contact sheet FlowPath
     * @returns report — Contact sheet image report
     * @impure has side effects / drives control flow
     */
    function contactSheet({ source: Struct, target: Struct, maxFrames?: int, everyNFrames?: int, columns?: int, cellWidth?: int, cellHeight?: int, videoTrackId?: int, format?: string }): { result: Struct, report: Struct };

    /**
     * Decode a video frame and write it as a still image
     * @node video_extract_thumbnail @alias videoExtractThumbnail
     * @param source — Source media FlowPath
     * @param target — Target image FlowPath
     * @param frameIndex (optional) — Decoded frame index to export
     * @param videoTrackId (optional) — Video track id, or 0 for default
     * @param format (optional) — Output image format, or auto from target extension
     * @param width (optional) — Output width, or 0 to keep decoded width
     * @param height (optional) — Output height, or 0 to keep decoded height
     * @returns result — Written image FlowPath
     * @returns report — Thumbnail report
     * @impure has side effects / drives control flow
     */
    function extractThumbnail({ source: Struct, target: Struct, frameIndex?: int, videoTrackId?: int, format?: string, width?: int, height?: int }): { result: Struct, report: Struct };

    // === Video/Tracks ===

    /**
     * Write one encoded media track into a new container
     * @node video_extract_track @alias videoExtractTrack
     * @param source — Source media FlowPath
     * @param target — Target media FlowPath
     * @param trackId (optional) — Track to keep
     * @returns result — Written media FlowPath
     * @returns packetCount — Packets written
     * @returns stream — Extracted stream metadata
     * @impure has side effects / drives control flow
     */
    function extractTrack({ source: Struct, target: Struct, trackId?: int }): { result: Struct, packetCount: int, stream: Struct };

    // === Video/Transcode ===

    /**
     * Decode a selected video stream and encode it to AV1 with the Rust rav1e backend
     * @node video_encode_av1 @alias videoEncodeAv1
     * @param source — Source media FlowPath
     * @param target — Target AV1 media FlowPath
     * @param videoTrackId (optional) — Video track id, or 0 for default
     * @param preserveNonVideo (optional) — Copy non-video packets when possible
     * @param speed (optional) — rav1e speed preset 0..10
     * @param quantizer (optional) — rav1e quantizer 0..255
     * @param maxKeyFrameInterval (optional) — Maximum keyframe interval
     * @param threads (optional) — Worker threads, or 0 for rav1e default
     * @returns result — Written AV1 media FlowPath
     * @returns report — AV1 encode report
     * @impure has side effects / drives control flow
     */
    function encodeAv1({ source: Struct, target: Struct, videoTrackId?: int, preserveNonVideo?: bool, speed?: int, quantizer?: int, maxKeyFrameInterval?: int, threads?: int }): { result: Struct, report: Struct };

    /**
     * Packet-copy when allowed or decode/encode a selected video stream into a target container
     * @node video_transcode_video @alias videoTranscodeVideo
     * @param source — Source media FlowPath
     * @param target — Target media FlowPath
     * @param outputCodec (optional) — Codec to encode, or copy to only packet-copy/remux
     * @param videoTrackId (optional) — Video track id, or 0 for default
     * @param allowPacketCopy (optional) — Use copy/remux when no encode stage is requested
     * @param preserveNonVideo (optional) — Copy compatible non-video packets
     * @param bitrate (optional) — Target bitrate in bits per second, or 0 for backend default
     * @returns result — Written media FlowPath
     * @returns report — Video transcode report
     * @impure has side effects / drives control flow
     */
    function transcode({ source: Struct, target: Struct, outputCodec?: string, videoTrackId?: int, allowPacketCopy?: bool, preserveNonVideo?: bool, bitrate?: int }): { result: Struct, report: Struct };

    /**
     * Decode video frames, apply frame transforms, encode, and mux the result
     * @node video_transform_video @alias videoTransformVideo
     * @param source — Source media FlowPath
     * @param target — Target media FlowPath
     * @param outputCodec (optional) — Video codec to encode, such as h264, h265, vp9, or av1
     * @param videoTrackId (optional) — Video track id, or 0 for default
     * @param preserveNonVideo (optional) — Copy non-video packets when possible
     * @param bitrate (optional) — Target bitrate in bits per second, or 0 for backend default
     * @param cropX (optional)
     * @param cropY (optional)
     * @param cropWidth (optional)
     * @param cropHeight (optional)
     * @param resizeWidth (optional)
     * @param resizeHeight (optional)
     * @param rotateDegrees (optional)
     * @param blurRadius (optional)
     * @param flipHorizontal (optional)
     * @param flipVertical (optional)
     * @param brightness (optional) — -1.0 to 1.0
     * @param contrast (optional) — 1.0 keeps contrast unchanged
     * @param saturation (optional) — 1.0 keeps saturation unchanged
     * @returns result — Written media FlowPath
     * @returns report — Video transform report
     * @impure has side effects / drives control flow
     */
    function transform({ source: Struct, target: Struct, outputCodec?: string, videoTrackId?: int, preserveNonVideo?: bool, bitrate?: int, cropX?: int, cropY?: int, cropWidth?: int, cropHeight?: int, resizeWidth?: int, resizeHeight?: int, rotateDegrees?: int, blurRadius?: int, flipHorizontal?: bool, flipVertical?: bool, brightness?: float, contrast?: float, saturation?: float }): { result: Struct, report: Struct };
}
