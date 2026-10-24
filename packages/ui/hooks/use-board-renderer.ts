"use client";

import { useSyncExternalStore } from "react";

export type BoardRenderer = "react-flow" | "egui";

export const BOARD_RENDERER_STORAGE_KEY = "flow-board-renderer";
const serverSnapshot = {
	renderer: "react-flow" as BoardRenderer,
	ready: false,
};
let snapshot = serverSnapshot;
let storageWindow: Window | undefined;
const listeners = new Set<() => void>();

const parseRenderer = (value: string | null): BoardRenderer =>
	value === "react-flow" ? "react-flow" : "egui";

function publish(renderer: BoardRenderer) {
	if (snapshot.ready && snapshot.renderer === renderer) return;
	snapshot = { renderer, ready: true };
	for (const listener of listeners) listener();
}

function onStorage(event: StorageEvent) {
	if (event.key !== BOARD_RENDERER_STORAGE_KEY && event.key !== null) return;
	try {
		if (event.storageArea && event.storageArea !== storageWindow?.localStorage)
			return;
	} catch {
		// A restricted browser can still deliver a preference update from another tab.
	}
	publish(parseRenderer(event.newValue));
}

function getSnapshot() {
	if (typeof window === "undefined") return serverSnapshot;
	if (storageWindow !== window) {
		storageWindow?.removeEventListener("storage", onStorage);
		storageWindow = window;
		let renderer: BoardRenderer = "egui";
		try {
			renderer = parseRenderer(
				window.localStorage.getItem(BOARD_RENDERER_STORAGE_KEY),
			);
		} catch {
			// Storage access is optional; the current browser session still works.
		}
		snapshot = { renderer, ready: true };
		window.addEventListener("storage", onStorage);
	}
	return snapshot;
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

/** Explicit choices persist when possible and always update mounted settings and boards. */
export function setBoardRendererPreference(renderer: BoardRenderer) {
	if (typeof window === "undefined") return;
	getSnapshot();
	try {
		window.localStorage.setItem(BOARD_RENDERER_STORAGE_KEY, renderer);
	} catch {
		// Keep the explicit choice in memory if persistence is blocked.
	}
	publish(renderer);
}

export function useBoardRenderer() {
	const preference = useSyncExternalStore(
		subscribe,
		getSnapshot,
		() => serverSnapshot,
	);
	return { ...preference, setRenderer: setBoardRendererPreference };
}
