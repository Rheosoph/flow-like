"use client";

import { useEffect, useRef, useState } from "react";
import { hexToRgbNorm } from "../color";
import { VOICE_DIMENSIONS, type VoiceVisualizerProps } from "../types";
import { Orb } from "./Orb";

const VERT = `
attribute vec2 a_pos;
void main() { gl_Position = vec4(a_pos, 0.0, 1.0); }
`;

const FRAG = `
precision mediump float;
uniform vec2 u_resolution;
uniform float u_time;
uniform float u_amp;
uniform float u_state;
uniform vec3 u_color;
uniform vec3 u_color2;

float hash(vec2 p) {
	p = fract(p * vec2(123.34, 456.21));
	p += dot(p, p + 45.32);
	return fract(p.x * p.y);
}

float noise(vec2 p) {
	vec2 i = floor(p);
	vec2 f = fract(p);
	float a = hash(i);
	float b = hash(i + vec2(1.0, 0.0));
	float c = hash(i + vec2(0.0, 1.0));
	float d = hash(i + vec2(1.0, 1.0));
	vec2 u = f * f * (3.0 - 2.0 * f);
	return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float fbm(vec2 p) {
	float v = 0.0;
	float amp = 0.5;
	for (int i = 0; i < 5; i++) {
		v += amp * noise(p);
		p *= 2.03;
		amp *= 0.5;
	}
	return v;
}

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

	// fresnel-style rim ring + soft outer bloom
	float rim = exp(-pow((d - radius) * 8.0, 2.0)) * (0.55 + amp);
	float glow = exp(-4.5 * max(0.0, d - radius)) * (0.35 + amp * 0.7);

	vec3 hot = mix(u_color, vec3(1.0), 0.65);
	vec3 col = mix(u_color2, u_color, clamp(n + 0.2, 0.0, 1.0));
	col = mix(col, hot, core * core * 0.7);

	vec3 finalCol = col * energy + u_color * glow + mix(u_color, hot, 0.5) * rim;
	float alpha = clamp(energy + glow * 0.7 + rim, 0.0, 1.0);

	// fade out before the canvas edge so nothing gets hard-clipped on the sides
	float mask = 1.0 - smoothstep(0.40, 0.5, d);
	finalCol *= mask;
	alpha *= mask;
	gl_FragColor = vec4(finalCol, alpha);
}
`;

function compile(
	gl: WebGLRenderingContext,
	type: number,
	src: string,
): WebGLShader | null {
	const shader = gl.createShader(type);
	if (!shader) return null;
	gl.shaderSource(shader, src);
	gl.compileShader(shader);
	if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
		gl.deleteShader(shader);
		return null;
	}
	return shader;
}

export function Shader(props: VoiceVisualizerProps) {
	const { analyser, state, size, color, recordingColor, hover } = props;
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const ampRef = useRef(0.04);
	const flowRef = useRef(0);
	const [failed, setFailed] = useState(false);
	const dim = VOICE_DIMENSIONS[size].orb;
	const main = state === "recording" ? recordingColor : color;

	useEffect(() => {
		if (failed) return;
		const canvas = canvasRef.current;
		if (!canvas) return;

		let gl: WebGLRenderingContext | null = null;
		try {
			gl = (canvas.getContext("webgl") ||
				canvas.getContext(
					"experimental-webgl",
				)) as WebGLRenderingContext | null;
		} catch {
			gl = null;
		}
		if (!gl) {
			setFailed(true);
			return;
		}

		const vert = compile(gl, gl.VERTEX_SHADER, VERT);
		const frag = compile(gl, gl.FRAGMENT_SHADER, FRAG);
		const program = gl.createProgram();
		if (!vert || !frag || !program) {
			setFailed(true);
			return;
		}
		gl.attachShader(program, vert);
		gl.attachShader(program, frag);
		gl.linkProgram(program);
		if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
			setFailed(true);
			return;
		}
		gl.useProgram(program);

		const buffer = gl.createBuffer();
		gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
		gl.bufferData(
			gl.ARRAY_BUFFER,
			new Float32Array([-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1]),
			gl.STATIC_DRAW,
		);
		const posLoc = gl.getAttribLocation(program, "a_pos");
		gl.enableVertexAttribArray(posLoc);
		gl.vertexAttribPointer(posLoc, 2, gl.FLOAT, false, 0, 0);
		gl.enable(gl.BLEND);
		gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

		const uResolution = gl.getUniformLocation(program, "u_resolution");
		const uTime = gl.getUniformLocation(program, "u_time");
		const uAmp = gl.getUniformLocation(program, "u_amp");
		const uState = gl.getUniformLocation(program, "u_state");
		const uColor = gl.getUniformLocation(program, "u_color");
		const uColor2 = gl.getUniformLocation(program, "u_color2");

		const dpr =
			typeof window !== "undefined"
				? Math.min(window.devicePixelRatio || 1, 2)
				: 1;
		canvas.width = dim * dpr;
		canvas.height = dim * dpr;
		gl.viewport(0, 0, canvas.width, canvas.height);

		const [r1, g1, b1] = hexToRgbNorm(main);
		const [r2, g2, b2] = hexToRgbNorm(state === "recording" ? color : main);

		let t = 0;
		let raf = 0;
		const dataArray = analyser
			? new Uint8Array(analyser.frequencyBinCount)
			: null;

		const render = () => {
			raf = requestAnimationFrame(render);
			if (!gl) return;

			// hover eases the plasma into flowing a little faster — a subtle
			// "it noticed you" reaction rather than an instant glow.
			const targetFlow = hover && state === "idle" ? 1 : 0;
			flowRef.current += (targetFlow - flowRef.current) * 0.05;
			t += 0.016 * (1 + flowRef.current * 0.9);

			const active = state === "recording" || state === "speaking";
			let target = 0.04 + flowRef.current * 0.07;
			if (analyser && dataArray && active) {
				analyser.getByteTimeDomainData(dataArray);
				let sum = 0;
				for (let i = 0; i < dataArray.length; i++) {
					const v = (dataArray[i] - 128) / 128;
					sum += v * v;
				}
				target = Math.max(
					target,
					Math.min(Math.sqrt(sum / dataArray.length) * 8, 1),
				);
			} else if (state === "processing") {
				target = 0.4 + Math.sin(t * 3) * 0.2;
			}
			if (state === "speaking") {
				target = Math.max(target, 0.4 + Math.sin(t * 3) * 0.18);
			}
			// ease the visible energy so nothing snaps on hover/state changes
			ampRef.current += (target - ampRef.current) * 0.08;
			const amplitude = ampRef.current;

			gl.uniform2f(uResolution, canvas.width, canvas.height);
			gl.uniform1f(uTime, t);
			gl.uniform1f(uAmp, amplitude);
			gl.uniform1f(uState, active ? 1 : state === "processing" ? 2 : 0);
			gl.uniform3f(uColor, r1, g1, b1);
			gl.uniform3f(uColor2, r2, g2, b2);
			gl.clearColor(0, 0, 0, 0);
			gl.clear(gl.COLOR_BUFFER_BIT);
			gl.drawArrays(gl.TRIANGLES, 0, 6);
		};
		render();

		return () => {
			cancelAnimationFrame(raf);
			gl.deleteProgram(program);
			gl.deleteShader(vert);
			gl.deleteShader(frag);
			gl.deleteBuffer(buffer);
		};
	}, [failed, analyser, state, main, color, dim, hover]);

	if (failed) return <Orb {...props} />;

	return <canvas ref={canvasRef} style={{ width: dim, height: dim }} />;
}
