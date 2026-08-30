// Streaming — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace video {
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
}
