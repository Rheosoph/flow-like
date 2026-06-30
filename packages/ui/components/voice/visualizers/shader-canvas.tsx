"use client";

import { useEffect, useRef, useState } from "react";
import { hexToRgbNorm } from "../color";
import { VOICE_DIMENSIONS, type VoiceVisualizerProps } from "../types";
import { Orb } from "./Orb";

const VERT = `
attribute vec2 a_pos;
void main() { gl_Position = vec4(a_pos, 0.0, 1.0); }
`;

/** Shared GLSL helpers every voice shader can rely on (value noise + fbm). */
export const SHADER_NOISE = `
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

export interface ShaderCanvasProps extends VoiceVisualizerProps {
	/**
	 * Fragment shader body. Receives uniforms: vec2 u_resolution, float u_time,
	 * float u_amp (0..1 eased audio energy), float u_state (0 idle, 1 active,
	 * 2 processing), vec3 u_color, vec3 u_color2. SHADER_NOISE helpers are
	 * available. Render into a circular area; the orb canvas is square.
	 */
	frag: string;
}

/**
 * Reusable WebGL canvas for orb-style voice visualizers: handles audio energy
 * easing, hover flow, the standard uniform set, and a graceful Orb fallback
 * when WebGL is unavailable. A new shader variant only needs to supply `frag`.
 *
 * The GL program is compiled once (per `dim`/`frag`); state/colour/hover/audio
 * are read from refs each frame so hovering or changing state never recompiles
 * the shader.
 */
export function ShaderCanvas({ frag, ...props }: ShaderCanvasProps) {
	const { analyser, state, size, color, recordingColor, hover } = props;
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const ampRef = useRef(0.04);
	const flowRef = useRef(0);
	const [failed, setFailed] = useState(false);
	const dim = VOICE_DIMENSIONS[size].orb;
	const main = state === "recording" ? recordingColor : color;

	// Per-frame inputs live in refs so the (expensive) GL setup effect doesn't
	// recompile the shader program on every hover / state change.
	const analyserRef = useRef(analyser);
	const stateRef = useRef(state);
	const mainRef = useRef(main);
	const colorRef = useRef(color);
	const hoverRef = useRef(hover);
	analyserRef.current = analyser;
	stateRef.current = state;
	mainRef.current = main;
	colorRef.current = color;
	hoverRef.current = hover;

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
		const fragShader = compile(gl, gl.FRAGMENT_SHADER, frag);
		const program = gl.createProgram();
		if (!vert || !fragShader || !program) {
			// Free any partially-created resources before bailing out.
			if (vert) gl.deleteShader(vert);
			if (fragShader) gl.deleteShader(fragShader);
			if (program) gl.deleteProgram(program);
			setFailed(true);
			return;
		}
		gl.attachShader(program, vert);
		gl.attachShader(program, fragShader);
		gl.linkProgram(program);
		if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
			gl.deleteShader(vert);
			gl.deleteShader(fragShader);
			gl.deleteProgram(program);
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

		let t = 0;
		let raf = 0;
		const dataArray = new Uint8Array(2048);

		const render = () => {
			raf = requestAnimationFrame(render);
			if (!gl) return;

			const state = stateRef.current;
			const analyser = analyserRef.current;
			const hover = hoverRef.current;
			const main = mainRef.current;
			const [r1, g1, b1] = hexToRgbNorm(main);
			const [r2, g2, b2] = hexToRgbNorm(
				state === "recording" ? colorRef.current : main,
			);

			// hover eases the plasma into flowing a little faster — a subtle
			// "it noticed you" reaction rather than an instant glow.
			const targetFlow = hover && state === "idle" ? 1 : 0;
			flowRef.current += (targetFlow - flowRef.current) * 0.05;
			t += 0.016 * (1 + flowRef.current * 0.9);

			const active = state === "recording" || state === "speaking";
			let target = 0.04 + flowRef.current * 0.07;
			if (analyser && active) {
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
			gl.deleteShader(fragShader);
			gl.deleteBuffer(buffer);
		};
	}, [failed, dim, frag]);

	if (failed) return <Orb {...props} />;

	return <canvas ref={canvasRef} style={{ width: dim, height: dim }} />;
}
