"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useRef } from "react";
import { resolveColorToRgb } from "../../../lib/chart-theme";
import {
	type IComposerActivityChannel,
	createTypingResponse,
	decayPerk,
} from "../../../lib/composer-activity";
import { cn } from "../../../lib/utils";

const POINT_COUNT = 520;
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));

/**
 * Extra turn at full typing, in radians per *second* — roughly triple the resting drift at 60Hz.
 * Kept in seconds even though the resting spin beside it is per-frame, so the typing response is
 * the same speed on a 120Hz display; matching the pre-existing constant would have doubled it.
 */
const TYPING_SPIN = 0.41;
/** How far writing tips the sphere toward the composer below it. */
const TYPING_PITCH = 0.16;

interface IChatEmptyOrbProps {
	/** Rendered diameter in px. */
	readonly size?: number;
	readonly className?: string;
	/** The composer whose draft the sphere answers. Omit for a mark that only follows the pointer. */
	readonly activity?: IComposerActivityChannel;
	readonly typingMotion?: boolean;
}

interface IOrbPoint {
	readonly x: number;
	readonly y: number;
	readonly z: number;
	readonly scale: number;
}

function buildSphere(): IOrbPoint[] {
	const points: IOrbPoint[] = [];
	for (let i = 0; i < POINT_COUNT; i++) {
		const y = 1 - (i / (POINT_COUNT - 1)) * 2;
		const radius = Math.sqrt(Math.max(0, 1 - y * y));
		const theta = i * GOLDEN_ANGLE;
		points.push({
			x: Math.cos(theta) * radius,
			y,
			z: Math.sin(theta) * radius,
			scale: 0.5 + ((i * 37) % 13) / 13,
		});
	}
	return points;
}

/**
 * The empty-state mark: a slowly rotating point sphere that leans toward the
 * pointer. It carries the invitation so the empty state needs no copy. With
 * `typingMotion` it also answers the composer — turning a little faster and
 * tipping toward the text while you write.
 */
export function ChatEmptyOrb({
	size = 260,
	className,
	activity,
	typingMotion = false,
}: IChatEmptyOrbProps) {
	const { t } = useTranslation("chat");
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const typingMotionRef = useRef(typingMotion);
	typingMotionRef.current = typingMotion;

	useEffect(() => {
		const canvas = canvasRef.current;
		const ctx = canvas?.getContext("2d");
		if (!canvas || !ctx) return;

		const reduceMotion = window.matchMedia(
			"(prefers-reduced-motion: reduce)",
		).matches;
		const dpr = Math.min(window.devicePixelRatio || 1, 2);
		canvas.width = size * dpr;
		canvas.height = size * dpr;
		ctx.scale(dpr, dpr);

		const points = buildSphere();

		let core: [number, number, number] = [251, 86, 45];
		let rim: [number, number, number] = [139, 92, 246];
		// Additive blending glows on a dark ground but blows out to white on a
		// light one, so the ground's luminance picks the compositing mode.
		let onLightGround = false;

		const readTheme = () => {
			const styles = window.getComputedStyle(canvas);
			core = resolveColorToRgb(styles.getPropertyValue("--primary"), core);
			rim = resolveColorToRgb(
				styles.getPropertyValue("--fl-chat-chart-2"),
				rim,
			);
			const [r, g, b] = resolveColorToRgb(
				styles.getPropertyValue("--background"),
				[11, 10, 12],
			);
			onLightGround = (r * 0.299 + g * 0.587 + b * 0.114) / 255 > 0.5;
		};
		readTheme();

		const themeObserver = new MutationObserver(readTheme);
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ["class", "data-theme", "style"],
		});

		let targetX = 0;
		let targetY = 0;
		let leanX = 0;
		let leanY = 0;
		const onPointerMove = (event: PointerEvent) => {
			targetX = (event.clientX / window.innerWidth - 0.5) * 0.55;
			targetY = (event.clientY / window.innerHeight - 0.5) * 0.35;
		};
		window.addEventListener("pointermove", onPointerMove, { passive: true });

		const center = size / 2;
		const radius = size * 0.35;
		let spin = 0;
		let frame = 0;
		const response = activity ? createTypingResponse(activity) : null;
		let perk = 0;
		let last = 0;

		const render = (ms?: number) => {
			// The resting motion keeps its original per-frame constants; only the typing response
			// is integrated in seconds, so it settles the same way on a 120Hz display.
			const dt = last && ms ? Math.min((ms - last) / 1000, 0.05) : 0.016;
			if (ms) last = ms;
			const step = response?.advance(
				typingMotionRef.current && !reduceMotion,
				dt,
			);
			const typing = step?.typing ?? 0;
			const fullness = step?.fullness ?? 0;
			perk = decayPerk(perk, step?.perked ?? false, dt);

			spin += reduceMotion ? 0 : 0.0032 + TYPING_SPIN * typing * dt;
			leanX += (targetX - leanX) * 0.05;
			leanY += (targetY - leanY) * 0.05;

			ctx.clearRect(0, 0, size, size);
			ctx.globalCompositeOperation = onLightGround ? "source-over" : "lighter";

			// A draft swells the sphere a touch and tips it down at the composer below it.
			const swell = radius * (1 + fullness * 0.03 + perk * 0.035);
			const yaw = spin + leanX;
			const pitch = Math.sin(spin * 0.7) * 0.22 + leanY + typing * TYPING_PITCH;
			const cosYaw = Math.cos(yaw);
			const sinYaw = Math.sin(yaw);
			const cosPitch = Math.cos(pitch);
			const sinPitch = Math.sin(pitch);

			for (const point of points) {
				const x = point.x * cosYaw - point.z * sinYaw;
				let z = point.x * sinYaw + point.z * cosYaw;
				const y = point.y * cosPitch - z * sinPitch;
				z = point.y * sinPitch + z * cosPitch;

				const perspective = 1 / (2.4 - z);
				const px = center + x * swell * perspective * 2.05;
				const py = center + y * swell * perspective * 2.05;
				const depth = (z + 1) / 2;
				const alpha = onLightGround
					? 0.12 + depth * depth * 0.78
					: 0.06 + depth * depth * 0.62;
				const blend = 1 - depth;

				ctx.beginPath();
				ctx.fillStyle = `rgba(${Math.round(core[0] + (rim[0] - core[0]) * blend)},${Math.round(
					core[1] + (rim[1] - core[1]) * blend,
				)},${Math.round(core[2] + (rim[2] - core[2]) * blend)},${alpha})`;
				ctx.arc(px, py, (0.6 + depth * 1.9) * point.scale, 0, Math.PI * 2);
				ctx.fill();
			}

			ctx.globalCompositeOperation = "source-over";
			if (!reduceMotion) frame = requestAnimationFrame(render);
		};

		render();

		return () => {
			cancelAnimationFrame(frame);
			themeObserver.disconnect();
			window.removeEventListener("pointermove", onPointerMove);
		};
	}, [size, activity]);

	return (
		<div
			className={cn("relative shrink-0", className)}
			style={{ width: size, height: size }}
			aria-hidden="true"
		>
			<div
				className="absolute rounded-full"
				style={{
					inset: size * 0.16,
					background:
						"radial-gradient(circle, color-mix(in oklch, var(--primary) 34%, transparent) 0%, transparent 66%)",
					filter: "blur(26px)",
				}}
			/>
			<canvas
				ref={canvasRef}
				className="relative block"
				style={{ width: size, height: size }}
			/>
			<div
				className="absolute rounded-full border motion-safe:animate-[fl-orb-breathe_5.5s_ease-in-out_infinite]"
				style={{
					inset: size * 0.075,
					borderColor: "color-mix(in oklch, var(--primary) 20%, transparent)",
				}}
			/>
		</div>
	);
}
