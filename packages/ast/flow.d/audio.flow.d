// Audio — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

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
