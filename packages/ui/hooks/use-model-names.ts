"use client";

import {
	type UseQueryResult,
	useQueries,
	useQuery,
} from "@tanstack/react-query";
import { useMemo } from "react";
import {
	type IModelIdentity,
	bitModelName,
	describeModel,
	isBitIdLike,
	matchModelBit,
} from "../lib/bit/model-display-name";
import type { IBit } from "../lib/schema";
import { useBackend } from "../state/backend-state";

const MODEL_NAME_STALE_MS = 60 * 60 * 1000;
const PROFILE_BITS_STALE_MS = 5 * 60 * 1000;

function uniqueModels(models: readonly (string | undefined | null)[]) {
	return Array.from(
		new Set(models.map((model) => model?.trim() ?? "").filter(Boolean)),
	).sort();
}

function bitsOf(results: UseQueryResult<IBit | null>[]) {
	return results.map((result) => result.data ?? null);
}

/**
 * Describes every model string a usage report names: id-shaped strings are looked up
 * as Bits, provider model names are matched against the models in the user's profile.
 * Unmatched strings still get a cleaned-up name, provider prefix and snapshot date.
 */
export function useModelIdentities(
	models: readonly (string | undefined | null)[],
): ReadonlyMap<string, IModelIdentity> {
	const backend = useBackend();
	const key = JSON.stringify(uniqueModels(models));
	const all = useMemo(() => JSON.parse(key) as string[], [key]);
	const ids = useMemo(() => all.filter(isBitIdLike), [all]);
	const named = all.length > ids.length;

	const idBits = useQueries({
		queries: ids.map((id) => ({
			queryKey: ["modelIdentityBit", id],
			// A deleted Bit or an offline hub is expected here; the raw id is the fallback.
			queryFn: async () => {
				try {
					return await backend.bitState.getBit(id);
				} catch {
					return null;
				}
			},
			staleTime: MODEL_NAME_STALE_MS,
			retry: false,
		})),
		combine: bitsOf,
	});
	const profileBits = useQuery({
		queryKey: ["modelIdentityProfileBits"],
		queryFn: async () => {
			try {
				return await backend.bitState.getProfileBits();
			} catch {
				return [];
			}
		},
		enabled: named,
		staleTime: PROFILE_BITS_STALE_MS,
		retry: false,
	});

	return useMemo(() => {
		const byId = new Map<string, IBit>();
		ids.forEach((id, index) => {
			const bit = idBits[index];
			if (bit) byId.set(id, bit);
		});
		const profile = Array.isArray(profileBits.data) ? profileBits.data : [];
		return new Map(
			all.map((model) => [
				model,
				describeModel(model, byId.get(model) ?? matchModelBit(model, profile)),
			]),
		);
	}, [all, ids, idBits, profileBits.data]);
}

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
