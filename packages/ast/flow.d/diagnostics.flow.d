// Diagnostics — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Diagnostics ===

/**
 * Choose the preferred compiled backend for a codec and operation
 * @param codec (optional) — Codec id such as h264, h265, av1, aac, or mp3
 * @param direction (optional) — decode or encode
 * @returns selection — Preferred backend selection
 * @returns support — Compiled codec support registry
 * @impure has side effects / drives control flow
 */
declare function videoPickCodecBackend({ codec?: string, direction?: string }): { selection: Struct, support: Struct[] };

/**
 * Report compiled video-utils-rs features and recommended codec backend lanes
 * @returns backends — Recommended codec backends
 * @returns features — Compiled video-utils-rs feature set
 * @impure has side effects / drives control flow
 */
declare function videoProbeCodecBackends(): { backends: Struct[], features: Struct };

/**
 * Check whether the current host can decode or encode a codec through native platform APIs
 * @param codec (optional) — Codec id such as h264, h265, av1, aac, or mp3
 * @param direction (optional) — decode or encode
 * @returns probe — Platform codec probe result
 * @impure has side effects / drives control flow
 */
declare function videoProbePlatformCodec({ codec?: string, direction?: string }): Struct;

