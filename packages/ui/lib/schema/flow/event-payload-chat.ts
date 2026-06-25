export type IVoiceChatMode = "disabled" | "stt" | "record";
export type IVoiceInvokeMode = "manual" | "hold" | "auto";
export type IVoiceVariant =
	| "conservative"
	| "waveform"
	| "orb"
	| "vortex"
	| "shader";
export type IVoiceSize = "sm" | "md" | "lg";
export type IVoicePlaybackMode = "text" | "audio" | "both";

export interface IVoiceConfig {
	mode?: IVoiceChatMode | null;
	invoke?: IVoiceInvokeMode | null;
	variant?: IVoiceVariant | null;
	size?: IVoiceSize | null;
	color?: string | null;
	recording_color?: string | null;
	playback?: IVoicePlaybackMode | null;
	max_duration?: number | null;
	auto_stop?: boolean | null;
}

export interface IEventPayloadChat {
	allow_file_upload?: boolean | null;
	allow_voice_input?: boolean | null;
	allow_voice_output?: boolean | null;
	allow_voice_mode?: boolean | null;
	voice?: IVoiceConfig | null;
	navigate_to_routes?: string[] | null;
	default_tools?: string[] | null;
	example_messages?: string[] | null;
	history_elements?: number | null;
	tools?: string[] | null;
	[property: string]: any;
}
