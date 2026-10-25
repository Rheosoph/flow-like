"use client";

/**
 * Background-tab dormancy for the web app. Agent browsers open a flow-like tab per step and never
 * close them, and a board tab costs hundreds of MB, so a board this tab left for another flow-like
 * tab gives its memory back until it is shown again.
 */
import { useEffect, useRef, useState } from "react";

const CHANNEL_NAME = "flow-like:tab-activity";
const VISIBLE = "visible";

let channel: BroadcastChannel | undefined;
let trackers = 0;
let otherTabVisibleAt = Number.NEGATIVE_INFINITY;
const otherTabListeners = new Set<() => void>();
let holds = 0;

function announceVisible() {
	if (document.visibilityState === "visible") channel?.postMessage(VISIBLE);
}

function receive(event: MessageEvent) {
	if (event.data !== VISIBLE) return;
	otherTabVisibleAt = Date.now();
	for (const listener of otherTabListeners) listener();
}

/** Tells the other tabs whenever this one is shown; the web app runs it on every page. */
export function trackTabActivity(): () => void {
	if (
		typeof document === "undefined" ||
		typeof BroadcastChannel === "undefined"
	)
		return () => {};
	if (trackers++ === 0) {
		channel = new BroadcastChannel(CHANNEL_NAME);
		channel.onmessage = receive;
		document.addEventListener("visibilitychange", announceVisible);
		announceVisible();
	}
	let released = false;
	return () => {
		if (released) return;
		released = true;
		if (--trackers > 0) return;
		document.removeEventListener("visibilitychange", announceVisible);
		channel?.close();
		channel = undefined;
	};
}

/** Keeps this tab awake while it holds work that unmounting would lose, e.g. an unapplied draft. */
export function holdTabDormancy(): () => void {
	holds++;
	let released = false;
	return () => {
		if (released) return;
		released = true;
		holds--;
	};
}

export interface TabDormancyOptions {
	/** How long the tab stays hidden before it may sleep. */
	afterMs: number;
	/** Asked when the tab would sleep; while true it asks again after another `afterMs`. */
	isBusy: () => boolean;
	onChange: (dormant: boolean) => void;
}

/**
 * Sleeps once this tab was hidden for `afterMs` and another flow-like tab was shown since, so a
 * user who only switched to another site keeps the board. Wakes as soon as the tab is shown.
 */
export function watchTabDormancy({
	afterMs,
	isBusy,
	onChange,
}: TabDormancyOptions): () => void {
	if (typeof document === "undefined") return () => {};
	const stopTracking = trackTabActivity();
	let hiddenAt: number | undefined;
	let dormant = false;
	let timer: ReturnType<typeof setTimeout> | undefined;
	const setDormant = (next: boolean) => {
		if (dormant === next) return;
		dormant = next;
		onChange(next);
	};
	const clearTimer = () => {
		if (timer !== undefined) clearTimeout(timer);
		timer = undefined;
	};
	const check = () => {
		clearTimer();
		if (hiddenAt === undefined || dormant) return;
		const wait = hiddenAt + afterMs - Date.now();
		if (wait > 0) {
			timer = setTimeout(check, wait);
			return;
		}
		// Left for another site, not another flow-like tab: the next tab shown checks again.
		if (otherTabVisibleAt < hiddenAt) return;
		if (holds > 0 || isBusy()) {
			timer = setTimeout(check, afterMs);
			return;
		}
		setDormant(true);
	};
	const onVisibility = () => {
		if (document.visibilityState === "visible") {
			hiddenAt = undefined;
			clearTimer();
			setDormant(false);
			return;
		}
		hiddenAt ??= Date.now();
		check();
	};
	document.addEventListener("visibilitychange", onVisibility);
	otherTabListeners.add(check);
	onVisibility();
	return () => {
		document.removeEventListener("visibilitychange", onVisibility);
		otherTabListeners.delete(check);
		clearTimer();
		stopTracking();
	};
}

export function useTabDormancy(afterMs: number, isBusy: () => boolean) {
	const [dormant, setDormant] = useState(false);
	const isBusyRef = useRef(isBusy);
	isBusyRef.current = isBusy;
	useEffect(
		() =>
			watchTabDormancy({
				afterMs,
				isBusy: () => isBusyRef.current(),
				onChange: setDormant,
			}),
		[afterMs],
	);
	return dormant;
}
