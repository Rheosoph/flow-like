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
	float spin = u_time * (0.18 + (u_state > 1.5 ? 0.5 : 0.0) + amp * 0.3);
	float ca = cos(spin);
	float sa = sin(spin);
	vec2 ruv = mat2(ca, -sa, sa, ca) * uv;

	float t = u_time * 0.4;
	// double domain-warp for organic, flowing plasma
	vec2 q = vec2(fbm(ruv * 2.2 + t), fbm(ruv * 2.2 - t + 5.2));
	vec2 w = vec2(
		fbm(ruv * 2.2 + q * 1.6 + t * 0.5),
		fbm(ruv * 2.2 + q * 1.6 - t * 0.5 + 3.3)
	);
	float n = fbm(ruv * 3.0 + w * (1.3 + amp * 1.8));

	float d = length(uv);
	float radius = 0.26 + amp * 0.1;
	float core = smoothstep(radius, 0.0, d);
	float filaments = pow(n, 1.6);
	float energy = core * (0.45 + amp * 0.8) + filaments * core * (1.4 + amp * 2.2);

	float rim = exp(-pow((d - radius) * 8.0, 2.0)) * (0.55 + amp);
	float glow = exp(-4.5 * max(0.0, d - radius)) * (0.35 + amp * 0.7);

	vec3 hot = mix(u_color, vec3(1.0), 0.65);
	vec3 col = mix(u_color2, u_color, clamp(n + 0.2, 0.0, 1.0));
	col = mix(col, hot, core * core * 0.7);

	vec3 finalCol = col * energy + u_color * glow + mix(u_color, hot, 0.5) * rim;
	float alpha = clamp(energy + glow * 0.7 + rim, 0.0, 1.0);

	float mask = 1.0 - smoothstep(0.40, 0.5, d);
	finalCol *= mask;
	alpha *= mask;
	gl_FragColor = vec4(finalCol, alpha);
}
`;

export function Shader(props: VoiceVisualizerProps) {
	return <ShaderCanvas frag={FRAG} {...props} />;
}
