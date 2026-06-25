"use client";

import { useEffect, useRef } from "react";
import { lighten, withAlpha } from "../color";
import { VOICE_DIMENSIONS, type VoiceVisualizerProps } from "../types";

const PARTICLES = 170;
const ARMS = 2; // two smooth arms = spiral galaxy (never a hooked cross)
const TURNS = 2.1;

interface Particle {
	t: number;
	baseAngle: number;
	wobblePhase: number;
	sizeJitter: number;
}

export function Vortex({
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
		const maxR = dim * 0.42;

		const particles: Particle[] = Array.from({ length: PARTICLES }, (_, i) => {
			const frac = (i + 1) / PARTICLES;
			const arm = i % ARMS;
			const jitterA = Math.sin(i * 12.9898) * 0.18;
			return {
				t: frac,
				baseAngle:
					frac * Math.PI * 2 * TURNS + (arm / ARMS) * Math.PI * 2 + jitterA,
				wobblePhase: (i * 7.13) % (Math.PI * 2),
				sizeJitter: 0.7 + ((i * 0.618) % 1) * 0.6,
			};
		});

		const draw = () => {
			animRef.current = requestAnimationFrame(draw);

			const active = state === "recording" || state === "speaking";
			const targetHover = hover && state === "idle" ? 1 : 0;
			hoverRef.current += (targetHover - hoverRef.current) * 0.06;
			let amp = 0.06 + hoverRef.current * 0.16;
			if (analyser && active) {
				const data = new Uint8Array(analyser.frequencyBinCount);
				analyser.getByteTimeDomainData(data);
				let sum = 0;
				for (let i = 0; i < data.length; i++) {
					const v = (data[i] - 128) / 128;
					sum += v * v;
				}
				amp = Math.max(amp, Math.min(Math.sqrt(sum / data.length) * 6, 1));
			} else if (state === "processing") {
				amp = 0.4 + Math.sin(phaseRef.current * 3) * 0.15;
			}
			if (state === "speaking") {
				amp = Math.max(amp, 0.4 + Math.sin(phaseRef.current * 3) * 0.18);
			}
			// single uniform rotation for every particle -> arms stay smooth
			phaseRef.current += 0.01 + amp * 0.03;
			const phase = phaseRef.current;

			// fade previous frame -> motion trails (transparent canvas)
			ctx.globalCompositeOperation = "destination-out";
			ctx.fillStyle = "rgba(0, 0, 0, 0.16)";
			ctx.fillRect(0, 0, dim, dim);

			ctx.globalCompositeOperation = "lighter";
			for (const p of particles) {
				const radius = p.t * maxR * (0.55 + amp * 0.55);
				const angle = p.baseAngle + phase;
				const wobble = Math.sin(phase * 1.5 + p.wobblePhase) * amp * 5;
				const x =
					cx + Math.cos(angle) * radius + Math.cos(angle + 1.57) * wobble;
				const y =
					cy + Math.sin(angle) * radius + Math.sin(angle + 1.57) * wobble;

				const psize = ((1 - p.t) * 2.4 + 0.7 + amp * 2) * p.sizeJitter * 3;
				const intensity = (0.32 + amp * 0.5) * (1 - p.t * 0.45);
				const g = ctx.createRadialGradient(x, y, 0, x, y, psize);
				g.addColorStop(0, lighten(main, 0.5 - p.t * 0.4, intensity));
				g.addColorStop(0.5, withAlpha(main, intensity * 0.6));
				g.addColorStop(1, withAlpha(main, 0));
				ctx.fillStyle = g;
				ctx.beginPath();
				ctx.arc(x, y, psize, 0, Math.PI * 2);
				ctx.fill();
			}

			// bright pulsing core
			const coreR = dim * 0.1 * (1 + amp * 0.9);
			const core = ctx.createRadialGradient(cx, cy, 0, cx, cy, coreR);
			core.addColorStop(0, lighten(main, 0.85, 0.55 + amp * 0.45));
			core.addColorStop(0.4, withAlpha(main, 0.7));
			core.addColorStop(1, withAlpha(main, 0));
			ctx.fillStyle = core;
			ctx.beginPath();
			ctx.arc(cx, cy, coreR, 0, Math.PI * 2);
			ctx.fill();

			ctx.globalCompositeOperation = "source-over";
		};
		draw();

		return () => cancelAnimationFrame(animRef.current);
	}, [analyser, state, main, dim, hover]);

	return <canvas ref={canvasRef} style={{ width: dim, height: dim }} />;
}
