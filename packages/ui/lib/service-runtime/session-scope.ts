const PREFIX = "device-runtime:";
const active = new Map<string, Map<string, unknown>>();
const used = new Set<string>();
const closeListeners = new Map<string, Set<() => void>>();

export const isRuntimeNamespace = (appId: unknown): appId is string =>
	typeof appId === "string" && appId.startsWith(PREFIX);

export const runtimeNamespaceActive = (appId: string) =>
	!isRuntimeNamespace(appId) || active.has(appId);

/** Namespaces are single-use so a callback from a closed view cannot reach a later view. */
export function openRuntimeNamespace(appId: string): () => void {
	if (!/^device-runtime:[A-Za-z0-9_-]{1,128}$/.test(appId) || used.has(appId))
		throw new Error("The deployed interface needs a new session identity.");
	used.add(appId);
	active.set(appId, new Map());
	return () => {
		active.get(appId)?.clear();
		active.delete(appId);
		for (const listener of closeListeners.get(appId) ?? []) listener();
		closeListeners.delete(appId);
	};
}

export function onRuntimeNamespaceClose(
	appId: string,
	callback: () => void,
): () => void {
	if (!runtimeNamespaceActive(appId)) {
		callback();
		return () => {};
	}
	let listeners = closeListeners.get(appId);
	if (!listeners) {
		listeners = new Set();
		closeListeners.set(appId, listeners);
	}
	listeners.add(callback);
	return () => {
		listeners.delete(callback);
	};
}

export function runtimeMemory(appId: string): Map<string, unknown> | undefined {
	return active.get(appId);
}

/** Chat database indexes use only the session ID, so runtime IDs include their app namespace. */
export function runtimeChatSessionId(appId: string, sessionId: string): string {
	if (!isRuntimeNamespace(appId)) return sessionId;
	const prefix = `${appId}:chat:`;
	return sessionId.startsWith(prefix) ? sessionId : `${prefix}${sessionId}`;
}

export function runtimeDomEventName(name: string, appId?: string): string {
	return isRuntimeNamespace(appId) ? `${name}:${appId}` : name;
}

export interface RuntimeNavigation {
	route: string;
	queryParams: Record<string, string>;
}

export function setRuntimeNavigation(
	appId: string,
	value: RuntimeNavigation,
): void {
	active.get(appId)?.set("navigation", {
		route: value.route,
		queryParams: { ...value.queryParams },
	} satisfies RuntimeNavigation);
}

export function runtimeNavigation(appId: string): RuntimeNavigation {
	return (
		(active.get(appId)?.get("navigation") as RuntimeNavigation | undefined) ?? {
			route: "/",
			queryParams: {},
		}
	);
}

/** Encoded chat rows carry their app identity inside their JSON payload. */
export function runtimeRowAppId(value: unknown): string | undefined {
	if (!value || typeof value !== "object") return undefined;
	const row = value as Record<string, unknown>;
	if (isRuntimeNamespace(row.appId)) return row.appId;
	if (typeof row.payload === "string" && row.payload.includes(PREFIX)) {
		try {
			const parsed = JSON.parse(row.payload);
			if (isRuntimeNamespace(parsed?.appId)) return parsed.appId;
		} catch {
			/* Malformed rows are handled by their normal database writer. */
		}
	}
	return undefined;
}
