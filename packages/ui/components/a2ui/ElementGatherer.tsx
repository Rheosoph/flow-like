"use client";

import { useCallback, useRef } from "react";
import type { A2UIComponent, BoundValue, Surface } from "./types";

export interface ElementState {
	path: string;
	value: unknown;
	timestamp: number;
}

export interface ElementGathererResult {
	elements: Record<string, ElementState>;
	requestedIds: string[];
}

export function useElementGatherer(surfaces: Map<string, Surface>) {
	const cacheRef = useRef<Map<string, ElementState>>(new Map());

	const gatherElements = useCallback(
		(requestedIds: string[]): ElementGathererResult => {
			const elements: Record<string, ElementState> = {};
			const now = Date.now();

			for (const id of requestedIds) {
				const cached = cacheRef.current.get(id);
				if (cached && now - cached.timestamp < 5000) {
					elements[id] = cached;
					continue;
				}

				const [surfaceId, ...componentPath] = id.split("/");
				const surface = surfaces.get(surfaceId);

				if (surface) {
					const value = resolveElementValue(surface, componentPath.join("/"));
					const state: ElementState = { path: id, value, timestamp: now };
					elements[id] = state;
					cacheRef.current.set(id, state);
				}
			}

			return { elements, requestedIds };
		},
		[surfaces],
	);

	const updateElement = useCallback((id: string, value: unknown) => {
		cacheRef.current.set(id, {
			path: id,
			value,
			timestamp: Date.now(),
		});
	}, []);

	const clearCache = useCallback(() => {
		cacheRef.current.clear();
	}, []);

	return { gatherElements, updateElement, clearCache };
}

function resolveElementValue(surface: Surface, componentPath: string): unknown {
	const surfaceComponent = surface.components?.[componentPath];
	if (!surfaceComponent) return undefined;

	return extractComponentValue(surfaceComponent.component);
}

function extractComponentValue(component: A2UIComponent): unknown {
	const comp = component as unknown as Record<string, unknown>;

	if ("value" in comp) {
		return resolveBoundValue(comp.value as BoundValue | string | undefined);
	}

	if ("checked" in comp) {
		return resolveBoundValue(comp.checked as BoundValue | boolean | undefined);
	}

	if ("content" in comp) {
		return resolveBoundValue(comp.content as BoundValue | string | undefined);
	}

	return undefined;
}

function resolveBoundValue(
	bound: BoundValue | string | number | boolean | null | undefined,
): unknown {
	if (bound === null || bound === undefined) return undefined;
	if (typeof bound !== "object") return bound;

	if ("literalString" in bound) {
		return bound.literalString;
	}

	if ("literalNumber" in bound) {
		return bound.literalNumber;
	}

	if ("literalBool" in bound) {
		return bound.literalBool;
	}

	return undefined;
}

export function createElementPayload(
	elements: Record<string, ElementState>,
): Record<string, unknown> {
	const payload: Record<string, unknown> = {};

	for (const [id, state] of Object.entries(elements)) {
		payload[id] = state.value;
	}

	return payload;
}
