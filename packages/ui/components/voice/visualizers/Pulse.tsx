"use client";

import { useEffect, useRef } from "react";
import { lighten, withAlpha } from "../color";
import { VOICE_DIMENSIONS, type VoiceVisualizerProps } from "../types";

const RINGS = 4;

/**
 * Sonar-style visualizer: a pulsing central core emits concentric rings that
 * expand and fade outward, their speed and brightness driven by audio energy.
 */
export function Pulse({
	analyser,
	state,
	size,
	color,
	recordingColor,
	hover,
}: VoiceVisualizerProps) {
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const animRef = useRef<number>(0);
	const phaseRef = useRef(0);
	const hoverRef = useRef(0);
	const dim = VOICE_DIMENSIONS[size].orb;
	const main = state === "recording" ? recordingColor : color;

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas) return;
		const ctx = canvas.getContext("2d");
		if (!ctx) return;

		const dpr =
			typeof window !== "undefined"
				? Math.min(window.devicePixelRatio || 1, 2)
				: 1;
		canvas.width = dim * dpr;
		canvas.height = dim * dpr;
		ctx.scale(dpr, dpr);

		const cx = dim / 2;
		const cy = dim / 2;
		const baseR = dim * 0.12;
		const maxR = dim * 0.46;

		const draw = () => {
			animRef.current = requestAnimationFrame(draw);
			ctx.clearRect(0, 0, dim, dim);
			phaseRef.current += 0.01;
			const phase = phaseRef.current;

			const active = state === "recording" || state === "speaking";
			let amplitude = 0;
			if (analyser && active) {
				const data = new Uint8Array(analyser.frequencyBinCount);
				analyser.getByteTimeDomainData(data);
				let sum = 0;
				for (let i = 0; i < data.length; i++) {
					const v = (data[i] - 128) / 128;
					sum += v * v;
				}
				amplitude = Math.sqrt(sum / data.length);
			}
			if (state === "speaking") {
				amplitude = Math.max(
					amplitude,
					0.05 + (Math.sin(phase * 3.2) * 0.5 + 0.5) * 0.07,
				);
			}
			if (state === "processing") {
				amplitude = Math.max(amplitude, 0.22 + Math.sin(phase * 4) * 0.12);
			}
			const targetHover = hover && state === "idle" ? 1 : 0;
			hoverRef.current += (targetHover - hoverRef.current) * 0.06;
			const idle = state === "idle" ? 0.03 + hoverRef.current * 0.06 : 0;
			const energy = Math.min(amplitude * 4 + idle, 1);

			// expanding rings
			ctx.globalCompositeOperation = "lighter";
			for (let i = 0; i < RINGS; i++) {
				const f = (((phase * (0.5 + energy * 1.4) + i / RINGS) % 1) + 1) % 1;
				const r = baseR + f * (maxR - baseR);
				const alpha = (1 - f) * (0.1 + energy * 0.45);
				ctx.beginPath();
				ctx.arc(cx, cy, r, 0, Math.PI * 2);
				ctx.strokeStyle = withAlpha(lighten(main, 0.3), alpha);
				ctx.lineWidth = 1.5 + energy * 2.5;
				ctx.stroke();
			}
			ctx.globalCompositeOperation = "source-over";

			// central pulsing core
			const coreR = baseR * (1 + energy * 0.7);
			const body = ctx.createRadialGradient(cx, cy, 0, cx, cy, coreR);
			body.addColorStop(0, lighten(main, 0.5, 0.96));
			body.addColorStop(0.6, withAlpha(main, 0.9));
			body.addColorStop(1, withAlpha(main, 0.45));
			ctx.fillStyle = body;
			ctx.beginPath();
			ctx.arc(cx, cy, coreR, 0, Math.PI * 2);
			ctx.fill();

			// soft glow
			ctx.globalCompositeOperation = "lighter";
			const glow = ctx.createRadialGradient(
				cx,
				cy,
				coreR * 0.5,
				cx,
				cy,
				coreR * 2.6,
			);
			glow.addColorStop(0, withAlpha(main, 0.25 + energy * 0.3));
			glow.addColorStop(1, withAlpha(main, 0));
			ctx.fillStyle = glow;
			ctx.beginPath();
			ctx.arc(cx, cy, coreR * 2.6, 0, Math.PI * 2);
			ctx.fill();
			ctx.globalCompositeOperation = "source-over";
		};
		draw();

		return () => cancelAnimationFrame(animRef.current);
	}, [analyser, state, main, dim, hover]);

	return <canvas ref={canvasRef} style={{ width: dim, height: dim }} />;
}
