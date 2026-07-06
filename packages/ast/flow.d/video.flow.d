// Video — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Video/Containers ===

/**
 * Rewrap compatible streams into another container without decoding
 * @param source — Source media FlowPath
 * @param target — Target media FlowPath
 * @returns result — Written media FlowPath
 * @returns report — Remux operation report
 * @impure has side effects / drives control flow
 */
declare function videoRemux({ source: Struct, target: Struct }): { result: Struct, report: Struct };


// === Video/Editing ===

/**
 * Concatenate packet-copy-compatible media files
 * @param sources — Media FlowPaths in concatenation order
 * @param target — Target media FlowPath
 * @returns result — Written media FlowPath
 * @returns packetCount — Packets written
 * @impure has side effects / drives control flow
 */
declare function videoConcat({ sources: Struct[], target: Struct }): { result: Struct, packetCount: int };

/**
 * Trim a media file using a keyframe-aligned packet range
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
declare function videoTrimKeyframes({ source: Struct, target: Struct, startSeconds?: float, endSeconds?: float, trackId?: int }): { result: Struct, packetCount: int, boundaryTrackId: int };


// === Video/Inspect ===

/**
 * Detect the media container for a FlowPath object
 * @param source — Media FlowPath to inspect
 * @returns container — Detected media container
 * @impure has side effects / drives control flow
 */
declare function videoDetectContainer({ source: Struct }): Struct;

/**
 * Extract stream metadata from a media FlowPath
 * @param source — Media FlowPath to inspect
 * @returns media — Container and stream metadata
 * @returns streams — Detected media streams
 * @impure has side effects / drives control flow
 */
declare function videoProbeMediaInfo({ source: Struct }): { media: Struct, streams: Struct[] };


// === Video/Packets ===

/**
 * Convert H.264/H.265/AAC packet bitstream framing into an elementary output file
 * @param source — Source media FlowPath
 * @param target — Target elementary FlowPath
 * @param conversion (optional) — h264_annex_b, h264_length_prefixed, h265_annex_b, h265_length_prefixed, aac_adts, or aac_raw
 * @param trackId (optional) — Track id, or 0 to select by conversion codec
 * @returns result — Written elementary FlowPath
 * @returns report — Bitstream conversion report
 * @impure has side effects / drives control flow
 */
declare function videoBitstreamConvert({ source: Struct, target: Struct, conversion?: string, trackId?: int }): { result: Struct, report: Struct };

/**
 * Rebase packet timestamps so each track starts at zero or later
 * @param source — Source media FlowPath
 * @param target — Target media FlowPath
 * @returns result — Written media FlowPath
 * @returns packetCount — Packets written
 * @impure has side effects / drives control flow
 */
declare function videoNormalizeTimestamps({ source: Struct, target: Struct }): { result: Struct, packetCount: int };


// === Video/Planning ===

/**
 * Check whether source streams can be packet-copied into a target container
 * @param source — Source media FlowPath
 * @param target — Target FlowPath with desired extension
 * @returns report — Detailed remux compatibility report
 * @impure has side effects / drives control flow
 */
declare function videoCheckRemuxCompatibility({ source: Struct, target: Struct }): Struct;


// === Video/Preview ===

/**
 * Sample decoded frames and write a preview grid image
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
declare function videoContactSheet({ source: Struct, target: Struct, maxFrames?: int, everyNFrames?: int, columns?: int, cellWidth?: int, cellHeight?: int, videoTrackId?: int, format?: string }): { result: Struct, report: Struct };

/**
 * Decode a video frame and write it as a still image
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
declare function videoExtractThumbnail({ source: Struct, target: Struct, frameIndex?: int, videoTrackId?: int, format?: string, width?: int, height?: int }): { result: Struct, report: Struct };


// === Video/Tracks ===

/**
 * Write one encoded media track into a new container
 * @param source — Source media FlowPath
 * @param target — Target media FlowPath
 * @param trackId (optional) — Track to keep
 * @returns result — Written media FlowPath
 * @returns packetCount — Packets written
 * @returns stream — Extracted stream metadata
 * @impure has side effects / drives control flow
 */
declare function videoExtractTrack({ source: Struct, target: Struct, trackId?: int }): { result: Struct, packetCount: int, stream: Struct };


// === Video/Transcode ===

/**
 * Decode a selected video stream and encode it to AV1 with the Rust rav1e backend
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
declare function videoEncodeAv1({ source: Struct, target: Struct, videoTrackId?: int, preserveNonVideo?: bool, speed?: int, quantizer?: int, maxKeyFrameInterval?: int, threads?: int }): { result: Struct, report: Struct };

/**
 * Packet-copy when allowed or decode/encode a selected video stream into a target container
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
declare function videoTranscodeVideo({ source: Struct, target: Struct, outputCodec?: string, videoTrackId?: int, allowPacketCopy?: bool, preserveNonVideo?: bool, bitrate?: int }): { result: Struct, report: Struct };

/**
 * Decode video frames, apply frame transforms, encode, and mux the result
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
declare function videoTransformVideo({ source: Struct, target: Struct, outputCodec?: string, videoTrackId?: int, preserveNonVideo?: bool, bitrate?: int, cropX?: int, cropY?: int, cropWidth?: int, cropHeight?: int, resizeWidth?: int, resizeHeight?: int, rotateDegrees?: int, blurRadius?: int, flipHorizontal?: bool, flipVertical?: bool, brightness?: float, contrast?: float, saturation?: float }): { result: Struct, report: Struct };

