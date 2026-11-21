"use client";

import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

interface IRect {
	readonly top: number;
	readonly left: number;
	readonly width: number;
	readonly height: number;
	readonly radius: number;
}

const OUTSET = 5;
const TAG_GAP = 12;

/** The first visible element marked `data-guide={target}`; targets are tour constants. */
export function findGuideTarget(target: string): HTMLElement | null {
	const candidates = document.querySelectorAll<HTMLElement>(
		`[data-guide="${target}"]`,
	);
	for (const element of candidates) {
		const rect = element.getBoundingClientRect();
		if (rect.width > 0 && rect.height > 0) return element;
	}
	return null;
}

function measure(element: HTMLElement): IRect {
	const rect = element.getBoundingClientRect();
	const radius = Number.parseFloat(
		getComputedStyle(element).borderTopLeftRadius,
	);
	return {
		top: rect.top - OUTSET,
		left: rect.left - OUTSET,
		width: rect.width + OUTSET * 2,
		height: rect.height + OUTSET * 2,
		radius: (Number.isFinite(radius) ? radius : 8) + OUTSET,
	};
}

function sameRect(a: IRect | null, b: IRect | null): boolean {
	return (
		a === b ||
		(!!a &&
			!!b &&
			a.top === b.top &&
			a.left === b.left &&
			a.width === b.width &&
			a.height === b.height &&
			a.radius === b.radius)
	);
}

/** Follows the element with `data-guide={target}`, wherever and whenever it renders. */
function useTargetRect(target: string | undefined): IRect | null {
	const [rect, setRect] = useState<IRect | null>(null);
	const last = useRef<IRect | null>(null);

	useEffect(() => {
		last.current = null;
		setRect(null);
		if (!target) return;
		let frame = 0;
		let element: HTMLElement | null = null;
		let lookedUpAt = 0;
		const tick = (now: number) => {
			if (!element?.isConnected || now - lookedUpAt > 500) {
				element = findGuideTarget(target);
				lookedUpAt = now;
			}
			const next = element ? measure(element) : null;
			if (!sameRect(next, last.current)) {
				last.current = next;
				setRect(next);
			}
			frame = requestAnimationFrame(tick);
		};
		frame = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(frame);
	}, [target]);

	return rect;
}

function tagPlacement(rect: IRect): "right" | "below" | "above" {
	const roomRight = window.innerWidth - (rect.left + rect.width);
	if (roomRight > 200 && rect.height < 120) return "right";
	if (window.innerHeight - (rect.top + rect.height) > 64) return "below";
	return "above";
}

export function GuideSpotlight({
	target,
	tag,
}: Readonly<{ target?: string; tag?: string }>) {
	const rect = useTargetRect(target);
	const ringRef = useRef<HTMLDivElement>(null);
	const shown = rect !== null;

	useEffect(() => {
		const ring = ringRef.current;
		if (!ring || !shown || typeof ring.animate !== "function") return;
		if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
		const animation = ring.animate(
			[
				{
					boxShadow:
						"0 0 0 2px var(--primary), 0 0 0 4px color-mix(in oklch, var(--primary) 12%, transparent)",
				},
				{
					boxShadow:
						"0 0 0 2px var(--primary), 0 0 0 10px color-mix(in oklch, var(--primary) 26%, transparent)",
				},
				{
					boxShadow:
						"0 0 0 2px var(--primary), 0 0 0 4px color-mix(in oklch, var(--primary) 12%, transparent)",
				},
			],
			{
				duration: 1600,
				iterations: Number.POSITIVE_INFINITY,
				easing: "ease-in-out",
			},
		);
		return () => animation.cancel();
	}, [shown]);

	if (!rect || typeof document === "undefined") return null;
	const placement = tagPlacement(rect);
	const tagStyle =
		placement === "right"
			? {
					top: rect.top + rect.height / 2,
					left: rect.left + rect.width + TAG_GAP,
					transform: "translateY(-50%)",
				}
			: placement === "below"
				? {
						top: rect.top + rect.height + TAG_GAP,
						left: rect.left + rect.width / 2,
						transform: "translateX(-50%)",
					}
				: {
						top: rect.top - TAG_GAP,
						left: rect.left + rect.width / 2,
						transform: "translate(-50%, -100%)",
					};

	return createPortal(
		<div
			aria-hidden="true"
			className="pointer-events-none fixed inset-0 z-[70]"
		>
			<div
				ref={ringRef}
				className="absolute transition-[top,left,width,height] duration-150"
				style={{
					top: rect.top,
					left: rect.left,
					width: rect.width,
					height: rect.height,
					borderRadius: rect.radius,
					boxShadow:
						"0 0 0 2px var(--primary), 0 0 0 6px color-mix(in oklch, var(--primary) 18%, transparent)",
				}}
			/>
			{tag && (
				<div
					className="absolute whitespace-nowrap rounded-md bg-primary px-2 py-1 font-semibold text-primary-foreground text-xs shadow-lg"
					style={tagStyle}
				>
					{tag}
				</div>
			)}
		</div>,
		document.body,
	);
}
