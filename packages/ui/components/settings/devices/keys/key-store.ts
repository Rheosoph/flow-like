"use client";

import { useCallback, useMemo } from "react";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import type { ResultTone } from "../primitives/inline-result";
import {
	useDeviceWorkspace,
	useManagerValue,
} from "../workspace/device-workspace-provider";

/* What this computer remembers about key backup files (P4): there is no hub record of a download. */

export interface KeyFileRecord {
	/** Epoch milliseconds a backup file was last saved from this computer. */
	savedAt?: number;
	/** Epoch milliseconds of the last password change here; older files open with the old password. */
	passwordChangedAt?: number;
}

export type KeyFileLog = Readonly<Record<string, KeyFileRecord>>;

const FILE_LOG_PREFIX = "flow-like/devices/key-files/";
const EMPTY_LOG: KeyFileLog = {};

interface FileLogStore {
	read(): KeyFileLog;
	update(deviceId: string, patch: KeyFileRecord | null): void;
	subscribe(listener: () => void): () => void;
}

function isRecord(value: unknown): value is KeyFileRecord {
	if (!value || typeof value !== "object") return false;
	const { savedAt, passwordChangedAt } = value as Record<string, unknown>;
	return (
		(savedAt === undefined || typeof savedAt === "number") &&
		(passwordChangedAt === undefined || typeof passwordChangedAt === "number")
	);
}

function loadLog(key: string): KeyFileLog {
	try {
		const raw = globalThis.localStorage?.getItem(key);
		if (!raw) return EMPTY_LOG;
		const parsed: unknown = JSON.parse(raw);
		if (!parsed || typeof parsed !== "object") return EMPTY_LOG;
		return Object.fromEntries(
			Object.entries(parsed).filter(([, value]) => isRecord(value)),
		);
	} catch {
		return EMPTY_LOG;
	}
}

function createFileLog(scopeKey: string): FileLogStore {
	const key = FILE_LOG_PREFIX + scopeKey;
	let log = loadLog(key);
	const listeners = new Set<() => void>();
	return {
		read: () => log,
		update(deviceId, patch) {
			const { [deviceId]: previous, ...rest } = log;
			log = patch ? { ...rest, [deviceId]: { ...previous, ...patch } } : rest;
			try {
				globalThis.localStorage?.setItem(key, JSON.stringify(log));
			} catch {
				// The record only feeds the "Backup file" column; this window keeps it.
			}
			for (const listener of [...listeners]) listener();
		},
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
	};
}

const fileLogs = new WeakMap<DeviceWorkspace, FileLogStore>();

export function keyFileLogOf(workspace: DeviceWorkspace): FileLogStore {
	let store = fileLogs.get(workspace);
	if (!store) {
		store = createFileLog(workspace.scopeKey);
		fileLogs.set(workspace, store);
	}
	return store;
}

/** When a backup file was last saved here, per device. */
export function useKeyFileLog(): KeyFileLog {
	const workspace = useDeviceWorkspace();
	const store = keyFileLogOf(workspace);
	const read = useCallback(() => store.read(), [store]);
	return useManagerValue(store.subscribe, read);
}

/* Results of key flows (R9): next to the row or block that started them, until dismissed. */

export interface KeyResult {
	/** `row:<deviceId>` or a block name. */
	scope: string;
	tone: ResultTone;
	text: string;
}

interface ResultStore {
	list(): readonly KeyResult[];
	put(result: KeyResult): void;
	dismiss(scope: string): void;
	subscribe(listener: () => void): () => void;
}

const MAX_RESULTS = 40;
const resultStores = new WeakMap<DeviceWorkspace, ResultStore>();

function createResults(): ResultStore {
	let results: readonly KeyResult[] = [];
	const listeners = new Set<() => void>();
	const set = (next: readonly KeyResult[]) => {
		results = next.slice(-MAX_RESULTS);
		for (const listener of [...listeners]) listener();
	};
	return {
		list: () => results,
		put: (result) =>
			set([...results.filter((row) => row.scope !== result.scope), result]),
		dismiss: (scope) => set(results.filter((row) => row.scope !== scope)),
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
	};
}

export function keyResultsOf(workspace: DeviceWorkspace): ResultStore {
	let store = resultStores.get(workspace);
	if (!store) {
		store = createResults();
		resultStores.set(workspace, store);
	}
	return store;
}

export const rowScope = (deviceId: string) => `row:${deviceId}`;

export interface KeyResults {
	of(scope: string): KeyResult | undefined;
	put(scope: string, tone: ResultTone, text: string): void;
	dismiss(scope: string): void;
}

/** One result per row or block; they live with the workspace, so a tab change keeps them. */
export function useKeyResults(): KeyResults {
	const workspace = useDeviceWorkspace();
	const store = keyResultsOf(workspace);
	const read = useCallback(() => store.list(), [store]);
	const results = useManagerValue(store.subscribe, read);
	return useMemo(
		() => ({
			of: (scope) => results.find((row) => row.scope === scope),
			put: (scope, tone, text) => store.put({ scope, tone, text }),
			dismiss: (scope) => store.dismiss(scope),
		}),
		[results, store],
	);
}
