"use client";

import { useEffect, useRef } from "react";
import { lighten, withAlpha } from "../color";
import { VOICE_DIMENSIONS, type VoiceVisualizerProps } from "../types";

const W = 320;

export function Waveform({
	analyser,
	state,
	size,
	color,
	recordingColor,
	hover,
}: VoiceVisualizerProps) {
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const animationRef = useRef<number>(0);
	const phaseRef = useRef(0);
	const hoverRef = useRef(0);
	const height = VOICE_DIMENSIONS[size].waveHeight;
	const stroke = state === "recording" ? recordingColor : color;

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas) return;
		const ctx = canvas.getContext("2d");
		if (!ctx) return;

		const dpr =
			typeof window !== "undefined"
				? Math.min(window.devicePixelRatio || 1, 2)
				: 1;
		canvas.width = W * dpr;
		canvas.height = height * dpr;
		ctx.scale(dpr, dpr);

		const active =
			(state === "recording" || state === "speaking") && !!analyser;
		const bufferLength = analyser ? analyser.frequencyBinCount : 512;
		const dataArray = new Uint8Array(bufferLength);
		const mid = height / 2;

		const gradient = ctx.createLinearGradient(0, 0, W, 0);
		gradient.addColorStop(0, withAlpha(stroke, 0.85));
		gradient.addColorStop(0.5, lighten(stroke, 0.45, 1));
		gradient.addColorStop(1, withAlpha(stroke, 0.85));

		const fill = ctx.createLinearGradient(0, 0, 0, height);
		fill.addColorStop(0, withAlpha(stroke, 0.28));
		fill.addColorStop(0.5, withAlpha(stroke, 0.04));
		fill.addColorStop(1, withAlpha(stroke, 0.28));

		const draw = () => {
			animationRef.current = requestAnimationFrame(draw);
			ctx.clearRect(0, 0, W, height);
			phaseRef.current += 0.05;

			if (active && analyser) {
				analyser.getByteTimeDomainData(dataArray);
			}

			const points = 96;
			const step = bufferLength / points;
			const sliceWidth = W / (points - 1);
			const targetHover = hover && !active ? 1 : 0;
			hoverRef.current += (targetHover - hoverRef.current) * 0.06;
			const idleAmp = 0.12 + hoverRef.current * 0.16;
			const speaking = state === "speaking";

			const sample = (i: number) => {
				const x = i / points;
				if (active) {
					const real = (dataArray[Math.floor(i * step)] - 128) / 128;
					// keep speaking lively even if playback audio can't be analysed
					if (speaking) {
						return (
							real +
							Math.sin(x * 7 + phaseRef.current * 1.6) *
								0.2 *
								Math.sin(x * Math.PI)
						);
					}
					return real;
				}
				// gentle idle ripple so it never reads as a dead line
				return (
					Math.sin(x * 9 + phaseRef.current) * idleAmp * Math.sin(x * Math.PI)
				);
			};

			// filled body
			ctx.beginPath();
			ctx.moveTo(0, mid);
			for (let i = 0; i < points; i++) {
				ctx.lineTo(i * sliceWidth, mid + sample(i) * mid * 0.92);
			}
			for (let i = points - 1; i >= 0; i--) {
				ctx.lineTo(i * sliceWidth, mid - sample(i) * mid * 0.92);
			}
			ctx.closePath();
			ctx.fillStyle = fill;
			ctx.fill();

			// glowing top stroke
			ctx.shadowBlur = active ? 12 : 6;
			ctx.shadowColor = withAlpha(stroke, 0.6);
			ctx.lineWidth = 2.5;
			ctx.lineJoin = "round";
			ctx.strokeStyle = gradient;
			ctx.beginPath();
			for (let i = 0; i < points; i++) {
				const y = mid + sample(i) * mid * 0.92;
				if (i === 0) ctx.moveTo(0, y);
				else ctx.lineTo(i * sliceWidth, y);
			}
			ctx.stroke();

			// faint mirror
			ctx.globalAlpha = 0.35;
			ctx.beginPath();
			for (let i = 0; i < points; i++) {
				const y = mid - sample(i) * mid * 0.92;
				if (i === 0) ctx.moveTo(0, y);
				else ctx.lineTo(i * sliceWidth, y);
			}
			ctx.stroke();
			ctx.globalAlpha = 1;
			ctx.shadowBlur = 0;
		};
		draw();

		return () => cancelAnimationFrame(animationRef.current);
	}, [analyser, state, stroke, height, hover]);

	return (
		<canvas ref={canvasRef} className="w-full rounded-lg" style={{ height }} />
	);
}
