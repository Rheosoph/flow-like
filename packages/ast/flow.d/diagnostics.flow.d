// Diagnostics — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

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
}
