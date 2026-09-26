"use client";

import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import {
	type PageSurfaceIdentity,
	readPageSurfaceCache,
	writePageSurfaceCache,
} from "../../lib/page-surface-cache";
import { applyA2UIMessage } from "../a2ui/apply-a2ui-message";
import { subscribeLivePageRuns } from "../a2ui/live-page-registry";
import type { A2UIServerMessage, Surface } from "../a2ui/types";
import { revealsPageLoad } from "./progressive-page-reveal";

/**
 * How long a load run's output must stop changing before it replaces the saved surface. A run
 * that keeps going after drawing its page (a live feed, a chat) would otherwise leave the old
 * content up for its whole lifetime.
 */
export const STALE_SURFACE_SETTLE_MS = 1500;

interface PageSurfaceCacheOptions {
	/** Cache identity of the page as it renders now. */
	readonly identity: PageSurfaceIdentity | null;
	/** The onLoad run whose output supersedes the saved surface. */
	readonly loadKey: string | null;
	/** False for noCache pages and pages without an onLoad run. */
	readonly enabled: boolean;
	/** The surface id action runs are reported under on the live-page bus. */
	readonly surfaceId: string | undefined;
	/** The current load run succeeded and nothing is loading. */
	readonly loadSucceeded: boolean;
	/** The surface the page's runs build; must be a stable callback. */
	readonly getLiveSurface: () => Surface | null;
}

export interface PageSurfaceCache {
	/**
	 * The surface an earlier visit ended on, shown while the load run rebuilds the page from its
	 * static layout. The run's messages never touch it, so a run that appends or clears starts
	 * from the layout it was written against. Null once the run's output takes over.
	 */
	readonly staleSurface: Surface | null;
	/** Mirrors a message from anything but the load run onto the saved surface on screen. */
	readonly applyToStale: (message: A2UIServerMessage) => void;
	/** The saved surface yields once load output settles or the run shows its screen. */
	readonly noteLoadOutput: (
		loadKey: string,
		message: A2UIServerMessage,
	) => void;
	/** Hands the screen to the live surface, e.g. once the load run has ended. */
	readonly releaseStale: (loadKey: string) => void;
	/** Saves the live surface; called after a run that changed it succeeded. */
	readonly persist: () => void;
}

/**
 * Stale-while-revalidate for a page whose content an onLoad run builds: the last surface is shown
 * at once, and only a successful run's result is ever written back.
 */
export function usePageSurfaceCache({
	identity,
	loadKey,
	enabled,
	surfaceId,
	loadSucceeded,
	getLiveSurface,
}: PageSurfaceCacheOptions): PageSurfaceCache {
	const [stale, setStale] = useState<{
		readonly loadKey: string;
		readonly surface: Surface;
	} | null>(null);
	// Set as the load run renders or ends, not at render, so a read landing in between still sees it.
	const supersededLoadKeyRef = useRef<string | null>(null);
	const settleTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(
		undefined,
	);
	const [persistRequest, persist] = useReducer((count: number) => count + 1, 0);

	const releaseStale = useCallback((key: string) => {
		supersededLoadKeyRef.current = key;
		clearTimeout(settleTimerRef.current);
		setStale((current) => (current?.loadKey === key ? null : current));
	}, []);

	useEffect(() => {
		if (!enabled || !identity || !loadKey) return;
		// A load key that comes back (a parameter toggled back) starts a new run.
		if (supersededLoadKeyRef.current === loadKey) {
			supersededLoadKeyRef.current = null;
		}
		let cancelled = false;
		void readPageSurfaceCache(identity).then((surface) => {
			if (cancelled || !surface) return;
			if (supersededLoadKeyRef.current === loadKey) return;
			setStale({ loadKey, surface });
		});
		return () => {
			cancelled = true;
		};
	}, [enabled, identity, loadKey]);

	useEffect(() => () => clearTimeout(settleTimerRef.current), []);

	const noteLoadOutput = useCallback(
		(key: string, message: A2UIServerMessage) => {
			if (!revealsPageLoad(message)) return;
			if (message.type === "showScreen") {
				releaseStale(key);
				return;
			}
			supersededLoadKeyRef.current = key;
			clearTimeout(settleTimerRef.current);
			settleTimerRef.current = setTimeout(
				() => releaseStale(key),
				STALE_SURFACE_SETTLE_MS,
			);
		},
		[releaseStale],
	);

	const applyToStale = useCallback((message: A2UIServerMessage) => {
		setStale((current) => {
			if (!current) return current;
			const surface = applyA2UIMessage(current.surface, message);
			return surface === current.surface ? current : { ...current, surface };
		});
	}, []);

	useEffect(() => {
		if (!enabled || !surfaceId) return;
		return subscribeLivePageRuns(surfaceId, (record) => {
			if (record.status === "ok") persist();
		});
	}, [enabled, surfaceId]);

	useEffect(() => {
		if (!persistRequest || !enabled || !identity || !loadSucceeded) return;
		const surface = getLiveSurface();
		if (surface) void writePageSurfaceCache(identity, surface);
	}, [persistRequest, enabled, identity, loadSucceeded, getLiveSurface]);

	return {
		staleSurface:
			enabled && stale && stale.loadKey === loadKey ? stale.surface : null,
		applyToStale,
		noteLoadOutput,
		releaseStale,
		persist,
	};
}
