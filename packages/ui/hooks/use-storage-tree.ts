"use client";

import { useQueries } from "@tanstack/react-query";
import { useCallback, useMemo, useState } from "react";
import type { IStorageItem } from "../lib/schema/storage/storage-item";
import {
	type IStorageScope,
	type IStorageTreeEntry,
	STORAGE_ROOT_PREFIX,
	normalizePrefix,
	sortStorageEntries,
	storagePrefixTrail,
	storageTreeEntry,
} from "../lib/storage-tree";
import { useBackend, useBackendReady } from "../state/backend-state";
import { useInvalidateInvoke } from "./use-invoke";

const STORAGE_TREE_STALE_MS = 30 * 1000;
const NO_PREFIXES: ReadonlySet<string> = new Set<string>();

type ListStorageItems = (
	appId: string,
	prefix: string,
) => Promise<IStorageItem[]>;

export interface IStorageDirectory {
	readonly prefix: string;
	readonly entries: readonly IStorageTreeEntry[];
	readonly isLoading: boolean;
	readonly isFetching: boolean;
	readonly error: Error | null;
}

export interface IStorageTree {
	/** Listed directories, keyed by their root-relative prefix. */
	readonly directories: ReadonlyMap<string, IStorageDirectory>;
	readonly root: IStorageDirectory;
	readonly expanded: ReadonlySet<string>;
	readonly isExpanded: (prefix: string) => boolean;
	readonly expand: (prefix: string) => void;
	readonly collapse: (prefix: string) => void;
	readonly toggle: (prefix: string) => void;
	readonly isLoading: boolean;
	readonly isFetching: boolean;
	/** Drops a directory's cache, or the whole tree's when no prefix is given. */
	readonly refetch: (prefix?: string) => Promise<void>;
}

export interface IStorageTreeOptions {
	readonly appId: string;
	readonly scope: IStorageScope;
	readonly enabled?: boolean;
}

const EMPTY_ROOT: IStorageDirectory = {
	prefix: STORAGE_ROOT_PREFIX,
	entries: [],
	isLoading: false,
	isFetching: false,
	error: null,
};

/**
 * Lazy listing of one storage scope: the root is always listed, every expanded
 * folder adds exactly one more listing. Query keys mirror `useInvoke`'s
 * `[fnName, ...args]` so a listing this tree loaded is the same cache entry the
 * file browser reads, and `useInvalidateInvoke` reaches both.
 */
export function useStorageTree({
	appId,
	scope,
	enabled = true,
}: IStorageTreeOptions): IStorageTree {
	const backend = useBackend();
	const backendReady = useBackendReady();
	const invalidate = useInvalidateInvoke();

	const scopeKey = `${scope}:${appId}`;
	const [expansion, setExpansion] = useState<{
		readonly key: string;
		readonly prefixes: ReadonlySet<string>;
	}>({ key: scopeKey, prefixes: NO_PREFIXES });

	// Expansion belongs to one scope: switching app or scope must not replay the
	// previous tree's prefixes against a namespace where they do not exist.
	const expanded =
		expansion.key === scopeKey ? expansion.prefixes : NO_PREFIXES;

	const listStorageItems: ListStorageItems =
		scope === "user"
			? backend.storageState.listStorageItemsUser
			: backend.storageState.listStorageItems;
	const storageState = backend.storageState;
	const queryName = listStorageItems.name || "backendFn";

	const prefixes = useMemo(
		() => [STORAGE_ROOT_PREFIX, ...Array.from(expanded).sort()],
		[expanded],
	);

	const canList = enabled && backendReady && appId.length > 0;

	const combine = useCallback(
		(
			results: readonly {
				data?: IStorageItem[];
				isLoading: boolean;
				isFetching: boolean;
				error: Error | null;
			}[],
		): readonly IStorageDirectory[] =>
			results.map((result, index) => {
				const prefix = prefixes[index];
				return {
					prefix,
					entries: sortStorageEntries(
						(result.data ?? []).map((item) =>
							storageTreeEntry(item, prefix, scope),
						),
					),
					isLoading: result.isLoading,
					isFetching: result.isFetching,
					error: result.error,
				};
			}),
		[prefixes, scope],
	);

	const listed = useQueries({
		queries: prefixes.map((prefix) => ({
			queryKey: [queryName, appId, prefix],
			queryFn: () => listStorageItems.call(storageState, appId, prefix),
			enabled: canList,
			staleTime: STORAGE_TREE_STALE_MS,
		})),
		combine,
	});

	const directories = useMemo(
		() =>
			new Map<string, IStorageDirectory>(
				listed.map((directory) => [directory.prefix, directory]),
			),
		[listed],
	);

	const updateExpanded = useCallback(
		(update: (current: ReadonlySet<string>) => ReadonlySet<string>) => {
			setExpansion((state) => {
				const current = state.key === scopeKey ? state.prefixes : NO_PREFIXES;
				const next = update(current);
				if (next === current && state.key === scopeKey) return state;
				return { key: scopeKey, prefixes: next };
			});
		},
		[scopeKey],
	);

	const expand = useCallback(
		(prefix: string) => {
			const trail = storagePrefixTrail(prefix).filter(Boolean);
			if (trail.length === 0) return;
			updateExpanded((current) => {
				if (trail.every((ancestor) => current.has(ancestor))) return current;
				const next = new Set(current);
				for (const ancestor of trail) next.add(ancestor);
				return next;
			});
		},
		[updateExpanded],
	);

	const collapse = useCallback(
		(prefix: string) => {
			const target = normalizePrefix(prefix);
			updateExpanded((current) => {
				if (!target) return current.size === 0 ? current : NO_PREFIXES;
				const next = new Set(
					Array.from(current).filter(
						(open) => open !== target && !open.startsWith(`${target}/`),
					),
				);
				return next.size === current.size ? current : next;
			});
		},
		[updateExpanded],
	);

	const isExpanded = useCallback(
		(prefix: string) => {
			const target = normalizePrefix(prefix);
			return target === STORAGE_ROOT_PREFIX || expanded.has(target);
		},
		[expanded],
	);

	const toggle = useCallback(
		(prefix: string) => {
			if (isExpanded(prefix)) collapse(prefix);
			else expand(prefix);
		},
		[collapse, expand, isExpanded],
	);

	const refetch = useCallback(
		async (prefix?: string) => {
			// Keyed on the app alone when no prefix is given: prefix matching then
			// also reaches listings that are still cached under a folder the user
			// has since collapsed, which would otherwise be replayed stale.
			const args =
				prefix === undefined ? [appId] : [appId, normalizePrefix(prefix)];
			await invalidate<IStorageItem[], string[]>(listStorageItems, args);
		},
		[appId, invalidate, listStorageItems],
	);

	return useMemo(
		() => ({
			directories,
			root: directories.get(STORAGE_ROOT_PREFIX) ?? EMPTY_ROOT,
			expanded,
			isExpanded,
			expand,
			collapse,
			toggle,
			isLoading: listed.some((directory) => directory.isLoading),
			isFetching: listed.some((directory) => directory.isFetching),
			refetch,
		}),
		[
			collapse,
			directories,
			expand,
			expanded,
			isExpanded,
			listed,
			refetch,
			toggle,
		],
	);
}
