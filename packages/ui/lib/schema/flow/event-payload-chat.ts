export interface IEventPayloadChat {
    /**
     * User-facing disclosure that the conversation is with an AI.
     */
    ai_disclosure?:     null | string;
    allow_file_upload?: boolean | null;
    /**
     * Show the model picker in the composer. Defaults to enabled.
     */
    allow_model_selection?: boolean | null;
    /**
     * Show the thinking picker in the composer. Defaults to enabled.
     */
    allow_thinking_selection?: boolean | null;
    allow_voice_input?:        boolean | null;
    allow_voice_mode?:         boolean | null;
    allow_voice_output?:       boolean | null;
    /**
     * Attach PNG snapshots of the latest assistant message's embedded widgets
     * to the outgoing user turn so vision-capable models see the rendered UI.
     * Defaults to enabled.
     */
    attach_widget_snapshots?: boolean | null;
    /**
     * Background image URL or app storage path for the chat interface.
     */
    background_image?: null | string;
    /**
     * Preferred chat color scheme: "system" | "light" | "dark".
     */
    color_scheme?: null | string;
    /**
     * Custom CSS scoped to the chat interface.
     */
    custom_css?: null | string;
    /**
     * Thinking level a new chat starts with: "off" | "low" | "mid" | "high".
     * Defaults to "low", the same level the Chat Event applies when none is sent.
     */
    default_thinking?: null | string;
    default_tools?:    string[] | null;
    example_messages?: string[] | null;
    history_elements?: number | null;
    /**
     * Which orb state the bubble placeholder rests in:
     * "idle" | "ready" | "thinking" | "working".
     */
    placeholder_bubble_state?: null | string;
    /**
     * Image URL or app storage path for the "image" placeholder.
     */
    placeholder_image?: null | string;
    /**
     * Let the placeholder mark react while the user types: it leans toward the
     * composer and stirs in proportion to how fast they write. Applies to the
     * "planet" and "bubble" marks. Defaults to disabled.
     */
    placeholder_typing_motion?: boolean | null;
    /**
     * Mark shown on the empty chat: "none" | "planet" | "bubble" | "image".
     * Defaults to "planet".
     */
    placeholder_visual?: null | string;
    /**
     * Offer only vision language models in the model picker. Defaults to disabled.
     */
    require_vlm?: boolean | null;
    tools?:       string[] | null;
    voice?:       null | IVoiceObject;
    [property: string]: any;
}

export interface IVoiceObject {
    auto_stop?: boolean | null;
    /**
     * Base accent color (CSS color string).
     */
    color?: null | string;
    /**
     * Invoke mode: "manual" | "hold" | "auto".
     */
    invoke?:       null | string;
    max_duration?: number | null;
    /**
     * Capture mode: "disabled" | "stt" (frontend transcript) | "record" (audio).
     */
    mode?: null | string;
    /**
     * Answer playback: "text" | "audio" | "both".
     */
    playback?: null | string;
    /**
     * Accent color while recording (CSS color string).
     */
    recording_color?: null | string;
    /**
     * Element size: "sm" | "md" | "lg".
     */
    size?: null | string;
    /**
     * Visual style: "conservative" | "waveform" | "orb" | "vortex" | "shader".
     */
    variant?: null | string;
    [property: string]: any;
}
