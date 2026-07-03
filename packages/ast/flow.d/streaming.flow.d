// Streaming — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Streaming ===

/**
 * Write an HLS media playlist plus MPEG-TS or fMP4 segments
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
declare function videoPackageHlsVod({ source: Struct, playlist: Struct, targetDurationSeconds?: float, segmentFormat?: string, segmentTrackId?: int, copyAllTracks?: bool, segmentPrefix?: string, initSegmentName?: string, uriPrefix?: string }): { playlistOut: Struct, segments: Struct[], report: Struct };

