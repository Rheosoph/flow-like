import type { IHub } from "../lib";
import { upstreamFailureInSuccess } from "../lib/api-error";
import { isRecord } from "../lib/response-shape";

type Listener = (hub: IHub | undefined) => void;
type Request = { controller: AbortController; completion: Promise<void> };
type HubEntry = {
	origin: string;
	listeners: Set<Listener>;
	hub?: IHub;
	freshUntil: number;
	pending?: Request;
};

// Availability stays in memory only while a consumer of this exact scope is mounted.
const entries = new Map<string, HubEntry>();
const HUB_FRESHNESS_MS = 30_000;

function fetchHub(entry: HubEntry, refresh = false): Promise<void> {
	if (entry.pending && !refresh) return entry.pending.completion;
	entry.pending?.controller.abort();
	const request: Request = {
		controller: new AbortController(),
		completion: Promise.resolve(),
	};
	entry.pending = request;
	const isCurrent = () =>
		!request.controller.signal.aborted && entry.pending === request;
	request.completion = (async () => {
		// The hub root has no trailing slash, matching hosted API routing.
		try {
			const response = await fetch(`${entry.origin}/api/v1`, {
				signal: request.controller.signal,
				cache: "no-store",
			});
			if (!isCurrent()) return;
			if (!response.ok) {
				console.error(
					`Hub config fetch returned ${response.status} from ${entry.origin}/api/v1`,
				);
				return;
			}
			const hub: unknown = await response.json();
			if (!isCurrent()) return;
			const upstreamError = upstreamFailureInSuccess(response, hub);
			if (upstreamError || !isRecord(hub)) {
				console.error(
					`Hub config fetch from ${entry.origin}/api/v1 returned no hub record`,
					upstreamError?.message,
				);
				return;
			}
			entry.hub = hub as unknown as IHub;
			entry.freshUntil = Date.now() + HUB_FRESHNESS_MS;
			for (const listener of entry.listeners) listener(entry.hub);
		} catch (error) {
			if (isCurrent()) console.error("Failed to fetch hub config:", error);
		} finally {
			if (entry.pending === request) entry.pending = undefined;
		}
	})();
	return request.completion;
}

export function subscribeToHub(
	scope: string,
	origin: string,
	listener: Listener,
) {
	let entry = entries.get(scope);
	if (!entry) {
		entry = { origin, listeners: new Set(), freshUntil: 0 };
		entries.set(scope, entry);
	}
	const shared = entry;
	shared.listeners.add(listener);
	listener(shared.hub);
	// Persistent shell consumers must not prevent a later page from refreshing.
	if (!shared.hub || shared.freshUntil <= Date.now()) void fetchHub(shared);
	let active = true;
	return {
		refetch: () => (active ? fetchHub(shared, true) : Promise.resolve()),
		release: () => {
			if (!active) return;
			active = false;
			shared.listeners.delete(listener);
			if (shared.listeners.size > 0) return;
			shared.pending?.controller.abort();
			entries.delete(scope);
		},
	};
}
