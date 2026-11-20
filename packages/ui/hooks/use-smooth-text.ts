"use client";

import { useEffect, useMemo, useRef, useState } from "react";

/** Time constant of the catch-up: the remaining gap shrinks by 63% per interval. */
const CATCH_UP_MS = 380;
/** Characters per second that never drop, so the last bit of a gap does not crawl. */
const MIN_RATE = 45;

const reducedMotion = () =>
	typeof window !== "undefined" &&
	typeof window.matchMedia === "function" &&
	window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * How many characters of `target` show after `dtMs` more, starting at `shown`. It speeds up with
 * the gap (it closes 63% of it per 380 ms, so nothing stays more than about a second behind),
 * and a trickle reads evenly.
 */
export function advanceSmoothText(
	shown: number,
	target: string,
	dtMs: number,
): number {
	const total = target.length;
	if (shown >= total) return total;
	const gap = total - shown;
	const rate = Math.max(MIN_RATE, (gap / CATCH_UP_MS) * 1_000);
	return Math.min(total, shown + (rate * dtMs) / 1_000);
}

function cut(text: string, length: number): string {
	let end = Math.floor(length);
	const code = text.charCodeAt(end);
	if (end > 0 && code >= 0xdc00 && code <= 0xdfff) end -= 1;
	return text.slice(0, end);
}

type Frame = (time: number) => void;
const schedule = (frame: Frame): (() => void) => {
	if (typeof requestAnimationFrame === "function") {
		const id = requestAnimationFrame(frame);
		return () => cancelAnimationFrame(id);
	}
	const id = setTimeout(() => frame(Date.now()), 16);
	return () => clearTimeout(id);
};

/**
 * Reveals the latest text of several growing replies between the updates that carry it. Texts
 * that are not in `targets` are not tracked; with reduced motion on, text shows at once.
 */
export function useSmoothTexts(
	targets: ReadonlyMap<string, string>,
): ReadonlyMap<string, string> {
	const shown = useRef(new Map<string, number>());
	const [tick, setTick] = useState(0);
	const latest = useRef(targets);
	latest.current = targets;
	const instant = reducedMotion();

	const behind = useMemo(() => {
		for (const id of [...shown.current.keys()])
			if (!targets.has(id)) shown.current.delete(id);
		for (const [id, text] of targets) {
			const have = shown.current.get(id);
			if (have === undefined) shown.current.set(id, 0);
			else if (have > text.length) shown.current.set(id, text.length);
		}
		return [...targets].some(
			([id, text]) => (shown.current.get(id) ?? 0) < text.length,
		);
	}, [targets]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: new targets restart a loop that already caught up
	useEffect(() => {
		if (!behind || instant) return;
		let last: number | undefined;
		let cancel = () => {};
		const frame: Frame = (time) => {
			const dt = last === undefined ? 16 : Math.min(100, time - last);
			last = time;
			let more = false;
			for (const [id, text] of latest.current) {
				const have = shown.current.get(id) ?? 0;
				const next = advanceSmoothText(have, text, dt);
				shown.current.set(id, next);
				if (next < text.length) more = true;
			}
			setTick((tick) => tick + 1);
			cancel = more ? schedule(frame) : () => {};
		};
		cancel = schedule(frame);
		return () => cancel();
	}, [behind, instant, targets]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: the frame tick re-reads the revealed lengths
	return useMemo(() => {
		const out = new Map<string, string>();
		for (const [id, text] of targets) {
			const length = instant ? text.length : (shown.current.get(id) ?? 0);
			out.set(id, length >= text.length ? text : cut(text, length));
		}
		return out;
	}, [targets, instant, tick]);
}
