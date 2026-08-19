import type { ILogMetadata } from "../../lib/schema/flow/log-metadata";
import type { Surface } from "./types";

/**
 * Module-scope registry of live, mounted app-page instances so FlowPilot tools can drive a
 * rendered page the way a user would: read its components, set input values, fire component
 * events, and observe the workflow runs those events start.
 *
 * PageInterface registers a handle via LivePageAgentBridge (mounted inside the page's
 * ActionProvider); the interact_app_page tool resolves handles through find/waitForLivePage.
 * Pattern precedent: widget-query-handler's module-scope registry.
 */

export interface LivePageRunRecord {
	/**
	 * ok = dispatched and no Error/Fatal logs; failed = dispatched but logged Error/Fatal;
	 * error = dispatch threw; not_executed = nothing ran (e.g. consent declined).
	 */
	status: "ok" | "failed" | "error" | "not_executed";
	runId?: string;
	componentId?: string;
	nodeId?: string;
	appId?: string;
	boardId?: string;
	errorMessage?: string;
	logMeta?: ILogMetadata;
	endedAtMs: number;
}

export interface LivePageTriggerResult {
	triggered: boolean;
	/** How the actions were resolved: exact handler, wildcard, legacy singleton, or none. */
	source: "event" | "wildcard" | "legacy" | "none";
	actionCount: number;
	runs: LivePageRunRecord[];
}

export interface LivePageHandle {
	appId: string;
	/** The page id — identical to the page surface id. */
	pageId: string;
	/** The page Event currently rendered (route/eventId target), when known. */
	eventId?: string;
	getSurface: () => Surface | null;
	/** The instance's own rendered page container, so captures shoot the DRIVEN instance. */
	getContainer?: () => HTMLElement | null;
	getElementValues: () => Record<string, unknown>;
	/** Writes both halves of an input value: the run-payload store and the visual surface. */
	setElementValue: (componentId: string, value: unknown) => void;
	/**
	 * Fire a component event through the page's real action pipeline and await every
	 * workflow run it starts. Runs are captured via notifyLivePageRun.
	 */
	triggerComponentEvent: (
		componentId: string,
		eventName: string,
	) => Promise<LivePageTriggerResult>;
	isLoading: () => boolean;
}

interface RegisteredLivePage {
	handle: LivePageHandle;
	registeredAt: number;
	instance: number;
}

const registry = new Map<number, RegisteredLivePage>();
let instanceCounter = 0;

export function registerLivePage(handle: LivePageHandle): () => void {
	instanceCounter += 1;
	const instance = instanceCounter;
	registry.set(instance, { handle, registeredAt: Date.now(), instance });
	return () => {
		registry.delete(instance);
	};
}

/**
 * Latest-registered live page matching the target. When both an inline card and the /use
 * route render the same page, the most recently mounted instance wins — both share the
 * same element-value storage, so driving either produces the same run payloads.
 */
export function findLivePage(
	appId: string,
	target: { eventId?: string; pageId?: string } = {},
): LivePageHandle | undefined {
	let best: RegisteredLivePage | undefined;
	for (const entry of registry.values()) {
		const { handle } = entry;
		if (handle.appId !== appId) continue;
		if (target.eventId && handle.eventId !== target.eventId) continue;
		if (target.pageId && handle.pageId !== target.pageId) continue;
		if (!best || entry.instance > best.instance) best = entry;
	}
	return best?.handle;
}

export async function waitForLivePage(
	appId: string,
	target: { eventId?: string; pageId?: string } = {},
	timeoutMs = 15_000,
): Promise<LivePageHandle | null> {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		const handle = findLivePage(appId, target);
		if (handle) return handle;
		await new Promise((resolve) => setTimeout(resolve, 100));
	}
	return findLivePage(appId, target) ?? null;
}

type RunListener = (record: LivePageRunRecord) => void;
const runListeners = new Map<string, Set<RunListener>>();

/**
 * Called by ActionHandler after a surface-triggered workflow run settles. `surfaceId` is
 * the page surface the triggering component belongs to.
 */
export function notifyLivePageRun(
	surfaceId: string | undefined,
	record: LivePageRunRecord,
): void {
	if (!surfaceId) return;
	const listeners = runListeners.get(surfaceId);
	if (!listeners) return;
	for (const listener of listeners) {
		try {
			listener(record);
		} catch {
			// A misbehaving listener must never break the page's action pipeline.
		}
	}
}

export function subscribeLivePageRuns(
	surfaceId: string,
	listener: RunListener,
): () => void {
	const listeners = runListeners.get(surfaceId) ?? new Set<RunListener>();
	listeners.add(listener);
	runListeners.set(surfaceId, listeners);
	return () => {
		listeners.delete(listener);
		if (listeners.size === 0) runListeners.delete(surfaceId);
	};
}
