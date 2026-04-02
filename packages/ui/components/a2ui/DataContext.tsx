"use client";

import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useMemo,
	useState,
} from "react";
import type { BoundValue, DataEntry } from "./types";

export interface DataContextValue {
	data: Record<string, unknown>;
	get: (path: string) => unknown;
	set: (path: string, value: unknown) => void;
	setByPath: (path: string, value: unknown) => void;
	resolve: (boundValue: BoundValue, defaultValue?: unknown) => unknown;
	update: (entries: DataEntry[]) => void;
	reset: (entries: DataEntry[]) => void;
}

export const DataContext = createContext<DataContextValue | null>(null);

function isUnsafeKey(key: string): boolean {
	return key === "__proto__" || key === "constructor" || key === "prototype";
}

function getByPath(obj: Record<string, unknown>, path: string): unknown {
	if (!path) return obj;
	const parts = path.split(".");
	let current: unknown = obj;

	for (const part of parts) {
		if (current === null || current === undefined) return undefined;
		if (typeof current !== "object") return undefined;

		const match = part.match(/^(\w+)\[(\d+)\]$/);
		if (match) {
			const [, key, index] = match;
			if (isUnsafeKey(key)) return undefined;
			const arr = (current as Record<string, unknown>)[key];
			if (!Array.isArray(arr)) return undefined;
			current = arr[Number.parseInt(index, 10)];
		} else {
			if (isUnsafeKey(part)) return undefined;
			current = (current as Record<string, unknown>)[part];
		}
	}

	return current;
}

function setByPath(
	obj: Record<string, unknown>,
	path: string,
	value: unknown,
): void {
	if (!path) return;
	const parts = path.split(".");
	let current: Record<string, unknown> = obj;

	for (let i = 0; i < parts.length - 1; i++) {
		const part = parts[i];
		const match = part.match(/^(\w+)\[(\d+)\]$/);

		if (match) {
			const [, key, index] = match;
			if (isUnsafeKey(key)) return;
			if (!current[key]) current[key] = [];
			const arr = current[key] as unknown[];
			const idx = Number.parseInt(index, 10);
			if (!arr[idx]) arr[idx] = {};
			current = arr[idx] as Record<string, unknown>;
		} else {
			if (isUnsafeKey(part)) return;
			if (!current[part]) current[part] = {};
			current = current[part] as Record<string, unknown>;
		}
	}

	const lastPart = parts[parts.length - 1];
	if (isUnsafeKey(lastPart)) return;
	const match = lastPart.match(/^(\w+)\[(\d+)\]$/);
	if (match) {
		const [, key, index] = match;
		if (isUnsafeKey(key)) return;
		if (!current[key]) current[key] = [];
		(current[key] as unknown[])[Number.parseInt(index, 10)] = value;
	} else {
		current[lastPart] = value;
	}
}

function deepClone<T>(obj: T): T {
	return JSON.parse(JSON.stringify(obj));
}

export function DataProvider({
	initialData,
	children,
}: {
	initialData: DataEntry[];
	children: ReactNode;
}) {
	const [data, setData] = useState<Record<string, unknown>>(() => {
		const initial: Record<string, unknown> = {};
		for (const entry of initialData) {
			setByPath(initial, entry.path, entry.value);
		}
		return initial;
	});

	const get = useCallback(
		(path: string): unknown => {
			return getByPath(data, path);
		},
		[data],
	);

	const set = useCallback((path: string, value: unknown): void => {
		setData((prev) => {
			const draft = deepClone(prev);
			setByPath(draft, path, value);
			return draft;
		});
	}, []);

	const resolve = useCallback(
		(boundValue: BoundValue, fallback?: unknown): unknown => {
			// Handle raw primitives passed directly (not wrapped in BoundValue)
			if (boundValue === null || boundValue === undefined)
				return fallback ?? boundValue;
			if (typeof boundValue !== "object") return boundValue;

			if ("literalString" in boundValue) return boundValue.literalString;
			if ("literalNumber" in boundValue) return boundValue.literalNumber;
			if ("literalBool" in boundValue) return boundValue.literalBool;
			if ("literalOptions" in boundValue) return boundValue.literalOptions;
			if ("literalJson" in boundValue) {
				try {
					return JSON.parse(boundValue.literalJson as string);
				} catch {
					return fallback;
				}
			}
			if ("path" in boundValue) {
				if (!boundValue.path) {
					return boundValue.defaultValue ?? fallback;
				}
				const value = get(boundValue.path);
				const defaultValue = boundValue.defaultValue ?? fallback;
				return value !== undefined ? value : defaultValue;
			}
			return fallback;
		},
		[get],
	);

	const update = useCallback((entries: DataEntry[]): void => {
		setData((prev) => {
			const draft = deepClone(prev);
			for (const entry of entries) {
				setByPath(draft, entry.path, entry.value);
			}
			return draft;
		});
	}, []);

	const reset = useCallback((entries: DataEntry[]): void => {
		const newData: Record<string, unknown> = {};
		for (const entry of entries) {
			setByPath(newData, entry.path, entry.value);
		}
		setData(newData);
	}, []);

	const value = useMemo(
		() => ({ data, get, set, setByPath: set, resolve, update, reset }),
		[data, get, set, resolve, update, reset],
	);

	return <DataContext.Provider value={value}>{children}</DataContext.Provider>;
}

const defaultContextValue: DataContextValue = {
	data: {},
	get: () => undefined,
	set: () => {},
	setByPath: () => {},
	resolve: (boundValue, defaultValue) => {
		if (boundValue === null || boundValue === undefined)
			return defaultValue ?? boundValue;
		if (typeof boundValue !== "object") return boundValue;
		if ("literalString" in boundValue) return boundValue.literalString;
		if ("literalNumber" in boundValue) return boundValue.literalNumber;
		if ("literalBool" in boundValue) return boundValue.literalBool;
		if ("literalOptions" in boundValue) return boundValue.literalOptions;
		if ("literalJson" in boundValue) {
			try {
				return JSON.parse(boundValue.literalJson as string);
			} catch {
				return defaultValue;
			}
		}
		if ("path" in boundValue) {
			return boundValue.defaultValue ?? defaultValue;
		}
		return defaultValue;
	},
	update: () => {},
	reset: () => {},
};

export function useData(): DataContextValue {
	const context = useContext(DataContext);
	if (!context) {
		console.warn("useData called outside DataProvider, using default context");
		return defaultContextValue;
	}
	return context;
}

export function useDataValue<T = unknown>(path: string): T {
	const { get } = useData();
	return get(path) as T;
}

export function useResolvedValue<T = unknown>(
	boundValue: BoundValue,
	defaultValue?: T,
): T {
	const { resolve } = useData();
	return resolve(boundValue, defaultValue) as T;
}
