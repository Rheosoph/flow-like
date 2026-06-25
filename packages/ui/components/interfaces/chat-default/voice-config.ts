import type { IEventPayloadChat } from "../../../lib";
import {
	DEFAULT_VOICE_CONFIG,
	type VoiceChatMode,
	type VoiceConfig,
	type VoiceInvokeMode,
	type VoicePlaybackMode,
	type VoiceSize,
	type VoiceVariant,
} from "../../voice";

/**
 * Maps the snake_case chat event voice config (and the legacy `allow_voice_*`
 * booleans) onto the shared camelCase {@link VoiceConfig}. Voice is disabled by
 * default when nothing is configured.
 */
export function resolveChatVoiceConfig(
	config?: Partial<IEventPayloadChat> | null,
): VoiceConfig {
	const v = config?.voice ?? null;
	const legacyMode: VoiceChatMode = config?.allow_voice_input
		? "record"
		: "disabled";
	const legacyPlayback: VoicePlaybackMode = config?.allow_voice_output
		? "audio"
		: "text";
	const legacyInvoke: VoiceInvokeMode = config?.allow_voice_mode
		? "auto"
		: DEFAULT_VOICE_CONFIG.invoke;

	return {
		...DEFAULT_VOICE_CONFIG,
		mode: (v?.mode as VoiceChatMode) ?? legacyMode,
		invoke: (v?.invoke as VoiceInvokeMode) ?? legacyInvoke,
		variant: (v?.variant as VoiceVariant) ?? DEFAULT_VOICE_CONFIG.variant,
		size: (v?.size as VoiceSize) ?? DEFAULT_VOICE_CONFIG.size,
		color: v?.color ?? DEFAULT_VOICE_CONFIG.color,
		recordingColor: v?.recording_color ?? DEFAULT_VOICE_CONFIG.recordingColor,
		playback: (v?.playback as VoicePlaybackMode) ?? legacyPlayback,
		maxDuration: v?.max_duration ?? DEFAULT_VOICE_CONFIG.maxDuration,
		autoStop: v?.auto_stop ?? DEFAULT_VOICE_CONFIG.autoStop,
	};
}

export function isVoiceEnabled(cfg: VoiceConfig): boolean {
	return cfg.mode !== "disabled";
}
