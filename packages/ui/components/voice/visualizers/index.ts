import type { ComponentType } from "react";
import type { VoiceVariant, VoiceVisualizerProps } from "../types";
import { Aurora } from "./Aurora";
import { Conservative } from "./Conservative";
import { Orb } from "./Orb";
import { Pulse } from "./Pulse";
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
	aurora: Aurora,
	pulse: Pulse,
};

export function getVoiceVisualizer(
	variant: VoiceVariant | string | undefined,
): ComponentType<VoiceVisualizerProps> {
	if (variant && variant in VOICE_VARIANTS) {
		return VOICE_VARIANTS[variant as VoiceVariant];
	}
	return VOICE_VARIANTS.waveform;
}

export { Aurora, Conservative, Orb, Pulse, Shader, Vortex, Waveform };
