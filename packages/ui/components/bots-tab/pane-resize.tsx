"use client";

import { useTranslation } from "@flow-like/locales";
import { GripVerticalIcon } from "lucide-react";
import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { cn } from "../../lib/utils";

const KEY_STEP_PX = 20;
const KEY_STEP_LARGE_PX = 80;

const clamp = (value: number, min: number, max: number) =>
	Math.min(Math.max(min, max), Math.max(min, value));

/** A pane width the reader dragged, kept in browser storage under `key`. */
export function useStoredWidth(
	key: string,
	initial: number,
	min: number,
	max: number,
) {
	const [width, setWidth] = useState(initial);
	useLayoutEffect(() => {
		try {
			const stored = Number(localStorage.getItem(key));
			if (Number.isFinite(stored) && stored >= min)
				setWidth(Math.min(max, stored));
		} catch {
			// Resizing still works when browser storage is unavailable.
		}
	}, [key, min, max]);
	const resize = useCallback(
		(next: number) => setWidth(clamp(next, min, max)),
		[min, max],
	);
	const save = useCallback(
		(value: number) => {
			try {
				localStorage.setItem(key, String(Math.round(clamp(value, min, max))));
			} catch {
				// The current layout does not depend on saving the preference.
			}
		},
		[key, min, max],
	);
	return { width, resize, save };
}

/**
 * A focusable splitter for the pane whose `edge` it sits on: dragging away from the pane widens it,
 * arrow keys step it, Home and End jump to the limits and a double click resets it. `className`
 * places it.
 */
export function PaneResizeHandle({
	edge,
	label,
	valueText,
	controls,
	width,
	min,
	max,
	initial,
	measure,
	onResize,
	onCommit,
	onDraggingChange,
	className,
}: Readonly<{
	/** The side of the pane the handle sits on. */
	edge: "left" | "right";
	label: string;
	valueText: string;
	controls?: string;
	width: number;
	min: number;
	max: number;
	/** Where a double click puts it back. */
	initial: number;
	/** The pane's width on screen when a drag starts. */
	measure: () => number;
	onResize: (width: number) => void;
	/** The width a drag or key press settled on, to remember. */
	onCommit: (width: number) => void;
	onDraggingChange?: (dragging: boolean) => void;
	className?: string;
}>) {
	const { t } = useTranslation("bots");
	const drag = useRef<{ x: number; width: number; last: number } | null>(null);
	const sign = edge === "left" ? -1 : 1;
	const set = (next: number) => {
		const bounded = clamp(next, min, max);
		onResize(bounded);
		return bounded;
	};
	const step = (next: number) => onCommit(set(next));
	return (
		// biome-ignore lint/a11y/useSemanticElements: This focusable window splitter has interactive children and range semantics.
		<div
			role="separator"
			tabIndex={0}
			aria-label={label}
			aria-orientation="vertical"
			aria-controls={controls}
			aria-valuemin={Math.round(min)}
			aria-valuemax={Math.round(Math.max(min, max))}
			aria-valuenow={Math.round(width)}
			aria-valuetext={valueText}
			title={t(
				"crew.dock.resizeHint",
				"Drag to resize. Double-click to reset.",
			)}
			className={cn(
				"group absolute inset-y-0 z-40 flex w-3 touch-none cursor-col-resize items-center justify-center outline-none",
				className,
			)}
			onPointerDown={(event) => {
				if (event.button !== 0) return;
				event.preventDefault();
				event.currentTarget.focus();
				event.currentTarget.setPointerCapture(event.pointerId);
				const start = measure();
				drag.current = { x: event.clientX, width: start, last: start };
				onDraggingChange?.(true);
			}}
			onPointerMove={(event) => {
				const current = drag.current;
				if (!current) return;
				current.last = set(current.width + sign * (event.clientX - current.x));
			}}
			onPointerUp={(event) => {
				if (event.currentTarget.hasPointerCapture(event.pointerId))
					event.currentTarget.releasePointerCapture(event.pointerId);
			}}
			onLostPointerCapture={() => {
				const current = drag.current;
				drag.current = null;
				if (current) onCommit(current.last);
				onDraggingChange?.(false);
			}}
			onDoubleClick={() => step(initial)}
			onKeyDown={(event) => {
				const amount = event.shiftKey ? KEY_STEP_LARGE_PX : KEY_STEP_PX;
				if (event.key === "ArrowLeft") step(width - sign * amount);
				else if (event.key === "ArrowRight") step(width + sign * amount);
				else if (event.key === "Home") step(min);
				else if (event.key === "End") step(max);
				else return;
				event.preventDefault();
			}}
		>
			<span className="absolute inset-y-0 w-0.5 transition-colors group-hover:bg-primary/50 group-focus-visible:bg-primary" />
			<span className="relative flex h-9 w-3 items-center justify-center rounded-full border bg-card text-muted-foreground opacity-0 transition-opacity group-hover:border-primary/50 group-hover:text-primary group-hover:opacity-100 group-focus-visible:opacity-100 group-focus-visible:ring-2 group-focus-visible:ring-ring">
				<GripVerticalIcon className="size-3" aria-hidden />
			</span>
		</div>
	);
}
