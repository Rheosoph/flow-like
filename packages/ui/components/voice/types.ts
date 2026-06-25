// Shared voice-input vocabulary + visual config used by both the default chat
// and the A2UI voiceInput element.

export type VoiceMode = "stt" | "record";
export type VoiceChatMode = "disabled" | VoiceMode;
export type VoiceInvokeMode = "hold" | "auto" | "manual";
export type VoiceVariant =
	| "conservative"
	| "waveform"
	| "orb"
	| "vortex"
	| "shader";
export type VoiceSize = "sm" | "md" | "lg";
export type VoicePlaybackMode = "text" | "audio" | "both";
export type VoiceVisualState = "idle" | "recording" | "processing" | "speaking";

export interface VoiceVisualizerProps {
	analyser: AnalyserNode | null;
	state: VoiceVisualState;
	size: VoiceSize;
	color: string;
	recordingColor: string;
	/** Pointer is hovering the control — adds idle liveliness. */
	hover?: boolean;
}

export const VOICE_DEFAULT_COLOR = "#8b5cf6";
export const VOICE_DEFAULT_RECORDING_COLOR = "#ef4444";

export const VOICE_DIMENSIONS: Record<
	VoiceSize,
	{ icon: number; orb: number; waveHeight: number }
> = {
	sm: { icon: 36, orb: 140, waveHeight: 48 },
	md: { icon: 56, orb: 220, waveHeight: 80 },
	lg: { icon: 72, orb: 300, waveHeight: 120 },
};

export const VOICE_VARIANTS_LIST: VoiceVariant[] = [
	"conservative",
	"waveform",
	"orb",
	"vortex",
	"shader",
];
export const VOICE_SIZES_LIST: VoiceSize[] = ["sm", "md", "lg"];

export interface VoiceConfig {
	mode: VoiceChatMode;
	invoke: VoiceInvokeMode;
	variant: VoiceVariant;
	size: VoiceSize;
	color: string;
	recordingColor: string;
	playback: VoicePlaybackMode;
	maxDuration?: number;
	autoStop?: boolean;
}

export const DEFAULT_VOICE_CONFIG: VoiceConfig = {
	mode: "disabled",
	invoke: "manual",
	variant: "conservative",
	size: "md",
	color: VOICE_DEFAULT_COLOR,
	recordingColor: VOICE_DEFAULT_RECORDING_COLOR,
	playback: "text",
	maxDuration: 300,
	autoStop: false,
};
