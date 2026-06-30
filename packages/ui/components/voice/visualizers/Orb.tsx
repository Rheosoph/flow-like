"use client";

import { useEffect, useRef } from "react";
import { lighten, withAlpha } from "../color";
import { VOICE_DIMENSIONS, type VoiceVisualizerProps } from "../types";

export function Orb({
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
	const hoverEaseRef = useRef(0);
	const dim = VOICE_DIMENSIONS[size].orb;
	const main = state === "recording" ? recordingColor : color;

	// Keep the per-frame inputs in refs so the canvas/animation effect doesn't
	// tear down and re-initialize on every hover/state change — it only needs to
	// re-run when the canvas size (`dim`) changes.
	const analyserRef = useRef(analyser);
	const stateRef = useRef(state);
	const mainRef = useRef(main);
	const hoverRef = useRef(hover);
	analyserRef.current = analyser;
	stateRef.current = state;
	mainRef.current = main;
	hoverRef.current = hover;

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
		const baseRadius = dim * 0.2;
		const maxBulge = dim * 0.14;

		const draw = () => {
			animRef.current = requestAnimationFrame(draw);
			const state = stateRef.current;
			const main = mainRef.current;
			const analyser = analyserRef.current;
			ctx.clearRect(0, 0, dim, dim);
			phaseRef.current += 0.02;
			const phase = phaseRef.current;

			const active = state === "recording" || state === "speaking";
			let amplitude = 0;
			if (analyser && active) {
				const dataArray = new Uint8Array(analyser.frequencyBinCount);
				analyser.getByteTimeDomainData(dataArray);
				let sum = 0;
				for (let i = 0; i < dataArray.length; i++) {
					const v = (dataArray[i] - 128) / 128;
					sum += v * v;
				}
				amplitude = Math.sqrt(sum / dataArray.length);
			}
			// keep speaking lively even if the playback analyser can't read audio
			if (state === "speaking") {
				amplitude = Math.max(
					amplitude,
					0.05 + (Math.sin(phase * 3.2) * 0.5 + 0.5) * 0.07,
				);
			}
			// idle breathing (eased-in on hover) so it feels alive but never pops
			const targetHover = hoverRef.current && state === "idle" ? 1 : 0;
			hoverEaseRef.current += (targetHover - hoverEaseRef.current) * 0.06;
			const idleAmt = state === "idle" ? 0.16 + hoverEaseRef.current * 0.18 : 0;
			const idle = (Math.sin(phase * 1.3) * 0.5 + 0.5) * idleAmt;
			const scaled = Math.min(amplitude * 8 + idle, 1);

			// additive outer bloom
			ctx.globalCompositeOperation = "lighter";
			const layers = [
				{
					radius: baseRadius + maxBulge * scaled,
					alpha: 0.1,
					blur: maxBulge * 2.2,
				},
				{
					radius: baseRadius + maxBulge * scaled * 0.7,
					alpha: 0.14,
					blur: maxBulge * 1.3,
				},
				{
					radius: baseRadius + maxBulge * scaled * 0.4,
					alpha: 0.22,
					blur: maxBulge * 0.6,
				},
			];
			for (const layer of layers) {
				const outer = layer.radius + layer.blur;
				const g = ctx.createRadialGradient(
					cx,
					cy,
					layer.radius * 0.3,
					cx,
					cy,
					outer,
				);
				g.addColorStop(0, withAlpha(main, layer.alpha));
				g.addColorStop(1, withAlpha(main, 0));
				ctx.fillStyle = g;
				ctx.beginPath();
				ctx.arc(cx, cy, outer, 0, Math.PI * 2);
				ctx.fill();
			}
			ctx.globalCompositeOperation = "source-over";

			// deformable body
			const points = 128;
			ctx.beginPath();
			for (let i = 0; i <= points; i++) {
				const angle = (i / points) * Math.PI * 2;
				const w1 = Math.sin(angle * 3 + phase * 2) * maxBulge * scaled * 0.32;
				const w2 = Math.sin(angle * 5 - phase * 1.5) * maxBulge * scaled * 0.22;
				const w3 = Math.sin(angle * 8 + phase * 3) * maxBulge * scaled * 0.12;
				const r = baseRadius + maxBulge * scaled * 0.5 + w1 + w2 + w3;
				const x = cx + Math.cos(angle) * r;
				const y = cy + Math.sin(angle) * r;
				if (i === 0) ctx.moveTo(x, y);
				else ctx.lineTo(x, y);
			}
			ctx.closePath();
			const body = ctx.createRadialGradient(
				cx - dim * 0.08,
				cy - dim * 0.08,
				0,
				cx,
				cy,
				baseRadius + maxBulge,
			);
			body.addColorStop(0, lighten(main, 0.5, 0.98));
			body.addColorStop(0.55, withAlpha(main, 0.92));
			body.addColorStop(1, withAlpha(main, 0.72));
			ctx.fillStyle = body;
			ctx.fill();

			// double rim light
			ctx.globalCompositeOperation = "lighter";
			ctx.lineWidth = 1.5;
			ctx.strokeStyle = lighten(main, 0.6, 0.35 + scaled * 0.4);
			ctx.stroke();
			ctx.globalCompositeOperation = "source-over";

			// rotating inner highlight
			const hx = cx + Math.cos(phase * 0.7) * baseRadius * 0.28;
			const hy = cy + Math.sin(phase * 0.7) * baseRadius * 0.28;
			const inner = ctx.createRadialGradient(
				hx,
				hy,
				0,
				hx,
				hy,
				baseRadius * 0.7,
			);
			inner.addColorStop(0, "rgba(255, 255, 255, 0.35)");
			inner.addColorStop(1, "rgba(255, 255, 255, 0)");
			ctx.fillStyle = inner;
			ctx.beginPath();
			ctx.arc(cx, cy, baseRadius * 0.85, 0, Math.PI * 2);
			ctx.fill();

			if (state === "processing") {
				ctx.strokeStyle = "rgba(255, 255, 255, 0.45)";
				ctx.lineWidth = 3;
				ctx.lineCap = "round";
				const spin = phase * 4;
				ctx.beginPath();
				ctx.arc(
					cx,
					cy,
					baseRadius + maxBulge * 0.6,
					spin,
					spin + Math.PI * 1.2,
				);
				ctx.stroke();
			}
		};
		draw();

		return () => cancelAnimationFrame(animRef.current);
	}, [dim]);

	return <canvas ref={canvasRef} style={{ width: dim, height: dim }} />;
}
