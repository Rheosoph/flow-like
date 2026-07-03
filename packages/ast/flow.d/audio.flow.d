// Audio — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Audio ===

/**
 * Decode audio and report waveform, peak/RMS, and silence ranges
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
declare function videoAnalyzeAudio({ source: Struct, waveformBuckets?: int, silenceThresholdDb?: float, windowMs?: int, minSilenceMs?: int }): { report: Struct, waveform: Struct[], silence: Struct[] };

/**
 * Decode an audio/media object and write WAV PCM output
 * @param source — Source audio/media FlowPath
 * @param target — Target WAV FlowPath
 * @param audioTrackId (optional) — Audio track id, or 0 for default
 * @returns result — Written WAV FlowPath
 * @returns report — Audio conversion report
 * @impure has side effects / drives control flow
 */
declare function videoAudioToWav({ source: Struct, target: Struct, audioTrackId?: int }): { result: Struct, report: Struct };

/**
 * Decode audio and return silence intervals
 * @param source — Source audio/media FlowPath
 * @param silenceThresholdDb (optional) — RMS threshold in dB
 * @param windowMs (optional) — Silence analysis window
 * @param minSilenceMs (optional) — Minimum silence duration
 * @returns silence — Detected silence ranges
 * @returns count — Detected silence range count
 * @impure has side effects / drives control flow
 */
declare function videoDetectSilence({ source: Struct, silenceThresholdDb?: float, windowMs?: int, minSilenceMs?: int }): { silence: Struct[], count: int };

/**
 * Decode audio, apply gain/normalization/fades, and write WAV PCM output
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
declare function videoTransformAudio({ source: Struct, target: Struct, audioTrackId?: int, gainFactor?: float, gainDb?: float, normalizePeak?: float, fadeInSeconds?: float, fadeOutSeconds?: float, fadeShape?: string }): { result: Struct, report: Struct };

