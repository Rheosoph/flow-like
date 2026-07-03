// Subtitles — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Subtitles ===

/**
 * Mux an SRT or WebVTT sidecar into a Matroska subtitle track
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
declare function videoAddSubtitleTrack({ source: Struct, sidecar: Struct, target: Struct, format?: string, trackId?: int, language?: string }): { result: Struct, report: Struct };

/**
 * Render an SRT/WebVTT sidecar into video frames and mux the result
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
declare function videoBurnSubtitles({ source: Struct, sidecar: Struct, target: Struct, format?: string, outputCodec?: string, videoTrackId?: int, preserveNonVideo?: bool, bitrate?: int, scale?: int, marginBottom?: int }): { result: Struct, report: Struct };

/**
 * Extract a subtitle track to an SRT or WebVTT sidecar
 * @param source — Source media FlowPath
 * @param target — Target sidecar FlowPath
 * @param format (optional) — Output subtitle format
 * @param trackId (optional) — Subtitle track id; 0 uses first subtitle track
 * @returns result — Written sidecar FlowPath
 * @returns report — Subtitle extraction report
 * @impure has side effects / drives control flow
 */
declare function videoExtractSubtitleTrack({ source: Struct, target: Struct, format?: string, trackId?: int }): { result: Struct, report: Struct };

/**
 * Parse SRT or WebVTT sidecar subtitles into cue structs
 * @param sidecar — Subtitle sidecar FlowPath
 * @param format (optional) — Subtitle format
 * @returns cues — Parsed subtitle cues
 * @returns count — Cue count
 * @impure has side effects / drives control flow
 */
declare function videoParseSubtitles({ sidecar: Struct, format?: string }): { cues: Struct[], count: int };

/**
 * Offset all SRT or WebVTT cues and write a new sidecar
 * @param source — Subtitle sidecar FlowPath
 * @param target — Target sidecar FlowPath
 * @param format (optional) — Subtitle format
 * @param offsetMs (optional) — Positive or negative subtitle offset in milliseconds
 * @returns result — Written sidecar FlowPath
 * @returns count — Shifted cue count
 * @impure has side effects / drives control flow
 */
declare function videoShiftSubtitleFile({ source: Struct, target: Struct, format?: string, offsetMs?: int }): { result: Struct, count: int };

/**
 * Write subtitle cue structs to an SRT or WebVTT sidecar
 * @param cues — Subtitle cues
 * @param target — Subtitle sidecar FlowPath
 * @param format (optional) — Subtitle format
 * @returns result — Written sidecar FlowPath
 * @returns count — Cue count
 * @impure has side effects / drives control flow
 */
declare function videoWriteSubtitles({ cues: Struct[], target: Struct, format?: string }): { result: Struct, count: int };

