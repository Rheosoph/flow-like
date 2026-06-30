"use client";

import type { VoiceVisualizerProps } from "../types";
import { SHADER_NOISE, ShaderCanvas } from "./shader-canvas";

const FRAG = `
precision mediump float;
uniform vec2 u_resolution;
uniform float u_time;
uniform float u_amp;
uniform float u_state;
uniform vec3 u_color;
uniform vec3 u_color2;
${SHADER_NOISE}
void main() {
	vec2 uv = (gl_FragCoord.xy - 0.5 * u_resolution.xy) / min(u_resolution.x, u_resolution.y);
	float amp = u_amp;
	float t = u_time * 0.35;

	// upward-drifting domain warp → flowing aurora curtains
	vec2 p = uv * 1.7;
	float warp = fbm(p * 1.4 + vec2(0.0, t * 1.2));
	float ribbons = fbm(p * 2.0 + vec2(warp * 1.5, -t * 1.6));

	// stacked horizontal curtains that wave, with fine vertical streaks
	float curtain = sin((uv.y * 3.2 + ribbons * 3.0 + t) * 3.14159);
	curtain = curtain * 0.5 + 0.5;
	float streak = 0.6 + 0.4 * fbm(vec2(uv.x * 9.0 + t, uv.y * 3.0 - t * 2.0));
	float energy = pow(curtain, 1.7) * streak * (0.5 + amp * 1.8) + ribbons * 0.25;

	float d = length(uv);
	float core = smoothstep(0.44, 0.05, d);
	energy *= core;

	vec3 col = mix(u_color2, u_color, clamp(ribbons * 0.7 + curtain * 0.5, 0.0, 1.0));
	vec3 hot = mix(u_color, vec3(1.0), 0.55);
	col = mix(col, hot, energy * energy * 0.5);

	float glow = exp(-4.0 * max(0.0, d - 0.18)) * (0.3 + amp * 0.7);
	vec3 finalCol = col * energy + u_color * glow;
	float alpha = clamp(energy + glow * 0.6, 0.0, 1.0);

	float mask = 1.0 - smoothstep(0.40, 0.5, d);
	finalCol *= mask;
	alpha *= mask;
	gl_FragColor = vec4(finalCol, alpha);
}
`;

export function Aurora(props: VoiceVisualizerProps) {
	return <ShaderCanvas frag={FRAG} {...props} />;
}
