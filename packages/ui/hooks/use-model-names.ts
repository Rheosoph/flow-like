"use client";

import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";
import { bitModelName, isBitIdLike } from "../lib/bit/model-display-name";
import { useBackend } from "../state/backend-state";

const MODEL_NAME_STALE_MS = 60 * 60 * 1000;

/**
 * Resolves the opaque Bit ids that providers report as their model name into the
 * Bit's catalog name, so usage stats and traces show "GPT-5.6" instead of
 * `tz4a98xxat96iws9zmbrgj3a`.
 *
 * Only id-shaped model strings are looked up; real model names pass through
 * untouched. Lookups are cached per id across every component that asks.
 */
export function useModelNames(
	models: readonly (string | undefined | null)[],
): ReadonlyMap<string, string> {
	const backend = useBackend();

	// Serialized rather than kept as an array: the caller rebuilds its model list
	// on every render, and only a change in the actual ids should retrigger work.
	const idKey = JSON.stringify(
		Array.from(
			new Set(
				models
					.map((model) => model?.trim() ?? "")
					.filter((model) => model && isBitIdLike(model)),
			),
		).sort(),
	);
	const ids = useMemo(() => JSON.parse(idKey) as string[], [idKey]);

	const resolvedKey = useQueries({
		queries: ids.map((id) => ({
			queryKey: ["bitModelName", id],
			// An unknown id is expected (deleted bit, offline hub) — fall back to the
			// raw id rather than surfacing a lookup failure inside a stats panel.
			queryFn: async () => {
				try {
					return bitModelName(await backend.bitState.getBit(id)) ?? "";
				} catch {
					return "";
				}
			},
			staleTime: MODEL_NAME_STALE_MS,
			retry: false,
		})),
		combine: (results) =>
			JSON.stringify(results.map((result) => result.data ?? "")),
	});

	return useMemo(() => {
		const names = new Map<string, string>();
		const resolved = JSON.parse(resolvedKey) as string[];
		ids.forEach((id, index) => {
			const name = resolved[index];
			if (name) names.set(id, name);
		});
		return names;
	}, [ids, resolvedKey]);
}
