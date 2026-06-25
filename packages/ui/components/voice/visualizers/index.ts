import type { ComponentType } from "react";
import type { VoiceVariant, VoiceVisualizerProps } from "../types";
import { Conservative } from "./Conservative";
import { Orb } from "./Orb";
import { Shader } from "./Shader";
import { Vortex } from "./Vortex";
import { Waveform } from "./Waveform";

export const VOICE_VARIANTS: Record<
	VoiceVariant,
	ComponentType<VoiceVisualizerProps>
> = {
	conservative: Conservative,
	waveform: Waveform,
	orb: Orb,
	vortex: Vortex,
	shader: Shader,
};

export function getVoiceVisualizer(
	variant: VoiceVariant | string | undefined,
): ComponentType<VoiceVisualizerProps> {
	if (variant && variant in VOICE_VARIANTS) {
		return VOICE_VARIANTS[variant as VoiceVariant];
	}
	return VOICE_VARIANTS.waveform;
}

export { Conservative, Orb, Shader, Vortex, Waveform };
