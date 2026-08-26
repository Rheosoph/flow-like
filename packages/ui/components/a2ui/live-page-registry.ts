import type { ILogMetadata } from "../../lib/schema/flow/log-metadata";
import type { A2UIComponent, Surface } from "./types";

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
	/** Resolves a BoundValue against the mounted page's current data model. */
	resolveBoundValue?: (value: unknown) => unknown;
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

const VALUE_BEARING_COMPONENT_TYPES = new Set<A2UIComponent["type"]>([
	"textField",
	"richText",
	"select",
	"slider",
	"checkbox",
	"switch",
	"radioGroup",
	"dateTimeInput",
]);

/** Components that can safely receive a JSON value through `set_value`. */
export function isLivePageValueBearingComponent(
	component: A2UIComponent,
): boolean {
	return VALUE_BEARING_COMPONENT_TYPES.has(component.type);
}

/** Component ids rendered below this component in the A2UI tree. */
export function livePageComponentChildIds(component: A2UIComponent): string[] {
	const children: string[] = [];
	if (component.children) {
		if ("explicitList" in component.children) {
			children.push(...component.children.explicitList);
		} else {
			children.push(component.children.template.templateComponentId);
		}
	}
	if (component.type === "overlay") {
		children.push(
			component.baseComponentId,
			...component.overlays.map((overlay) => overlay.componentId),
		);
	} else if (component.type === "tabs") {
		children.push(...component.tabs.map((tab) => tab.contentComponentId));
	} else if (component.type === "accordion") {
		children.push(...component.items.map((item) => item.contentComponentId));
	} else if (component.type === "popover") {
		children.push(component.contentComponentId);
	}
	return [...new Set(children.filter(Boolean))];
}

/** A hidden ancestor makes a descendant unreachable even when its own `hidden` field is false. */
export function isLivePageComponentEffectivelyHidden(
	surface: Surface,
	componentId: string,
	resolve: (value: unknown) => unknown,
): boolean {
	const parentById = new Map<string, string>();
	for (const [candidateId, entry] of Object.entries(surface.components ?? {})) {
		for (const childId of livePageComponentChildIds(entry.component)) {
			if (!parentById.has(childId)) parentById.set(childId, candidateId);
		}
	}

	const visited = new Set<string>();
	let currentId: string | undefined = componentId;
	while (currentId && !visited.has(currentId)) {
		visited.add(currentId);
		const component = surface.components?.[currentId]?.component;
		if (!component) break;
		if (
			resolve((component as A2UIComponent & Record<string, unknown>).hidden)
		) {
			return true;
		}
		currentId = parentById.get(currentId);
	}
	return false;
}

/**
 * Accept either the component id itself or the page-scoped `page/component` reference returned
 * by semantic inspection. A reference scoped to a different page retargets to this page's
 * component of the same name when one exists; a prefix naming a widget instance keeps its
 * widget semantics and is never retargeted.
 */
export function resolveLivePageComponentId(
	pageId: string,
	surface: Surface,
	requestedId: string,
): string {
	const requested = requestedId.trim();
	if (!requested) throw new Error("component_id is required.");

	// Preserve an exact component id first. This keeps hand-authored ids containing slashes valid.
	if (surface.components?.[requested]?.component) return requested;

	for (const scope of new Set([pageId, surface.id])) {
		const prefix = `${scope}/`;
		if (!requested.startsWith(prefix)) continue;
		const componentId = requested.slice(prefix.length);
		if (!componentId) {
			throw new Error(
				`Component reference '${requested}' has no component id.`,
			);
		}
		return componentId;
	}

	if (requested.includes("/")) {
		const slashIndex = requested.indexOf("/");
		const prefix = requested.slice(0, slashIndex);
		const componentId = requested.slice(slashIndex + 1);
		const prefixIsWidgetHost =
			surface.components?.[prefix]?.component?.type === "widgetInstance";
		if (
			!prefixIsWidgetHost &&
			componentId &&
			surface.components?.[componentId]?.component
		) {
			return componentId;
		}
		throw new Error(
			`Component reference '${requested}' belongs to a different page than '${pageId}', and this page has no component '${componentId}'.`,
		);
	}
	return requested;
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
