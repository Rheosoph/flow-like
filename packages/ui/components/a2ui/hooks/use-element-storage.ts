"use client";

import { useCallback, useEffect, useRef } from "react";
import {
	uiElementValues as elementValues,
	pruneElementValues,
} from "../../../db/ui-state-db";

/**
 * How long a value a user typed is worth restoring. Long enough to survive navigating away and
 * back, or a reload; short enough that a workflow is never handed something from another day.
 */
export const ELEMENT_VALUE_MAX_AGE_MS = 12 * 60 * 60 * 1000;

/**
 * Persistence for the values a surface's inputs are holding.
 *
 * These values are what `_elements` carries into a workflow. They used to live only in memory,
 * so a page that came back from its cached surface still looked filled in while every workflow
 * it ran received nothing — the screen and the payload disagreed. Persisting them per surface
 * keeps the two in step.
 */
export function useElementStorage(appId: string | undefined) {
	const pendingUpdates = useRef<
		Map<string, { value: unknown; timeout: NodeJS.Timeout }>
	>(new Map());

	// Store an element value (debounced to avoid excessive writes)
	const storeElementValue = useCallback(
		(elementId: string, value: unknown) => {
			if (!appId) return;

			// Cancel any pending update for this element
			const pending = pendingUpdates.current.get(elementId);
			if (pending) {
				clearTimeout(pending.timeout);
			}

			// Debounce the write by 300ms
			const timeout = setTimeout(() => {
				elementValues.set(appId, elementId, value).catch((err) => {
					console.error("[ElementStorage] Failed to store element value:", err);
				});
				pendingUpdates.current.delete(elementId);
			}, 300);

			pendingUpdates.current.set(elementId, { value, timeout });
		},
		[appId],
	);

	// Get a single element value
	const getElementValue = useCallback(
		async <T>(elementId: string): Promise<T | null> => {
			if (!appId) return null;
			const value = await elementValues.getValue<T>(appId, elementId);
			return value ?? null;
		},
		[appId],
	);

	// Get all element values for the app
	const getAllElementValues = useCallback(async (): Promise<
		Record<string, unknown>
	> => {
		if (!appId) return {};
		return elementValues.getAllValues(appId);
	}, [appId]);

	/**
	 * The values belonging to one surface, dropping anything old enough that restoring it would
	 * be a surprise rather than a continuation. Expired records are cleared as they are found,
	 * which is what keeps this store from growing without bound.
	 */
	const restoreSurfaceValues = useCallback(
		async (surfaceId: string): Promise<Record<string, unknown>> => {
			if (!appId || !surfaceId) return {};

			void pruneElementValues(appId, ELEMENT_VALUE_MAX_AGE_MS).catch(() => {});

			const all = await elementValues.getAll(appId);
			const cutoff = Date.now() - ELEMENT_VALUE_MAX_AGE_MS;
			const prefix = `${surfaceId}/`;
			const restored: Record<string, unknown> = {};

			for (const [elementId, record] of Object.entries(all)) {
				if (!elementId.startsWith(prefix)) continue;
				if (record.updatedAt < cutoff) continue;
				restored[elementId] = record.value;
			}

			return restored;
		},
		[appId],
	);

	// Clear all pending updates on unmount
	useEffect(() => {
		return () => {
			for (const pending of pendingUpdates.current.values()) {
				clearTimeout(pending.timeout);
			}
			pendingUpdates.current.clear();
		};
	}, []);

	return {
		storeElementValue,
		getElementValue,
		getAllElementValues,
		restoreSurfaceValues,
	};
}
