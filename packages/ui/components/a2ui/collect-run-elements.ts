import type { BoardVersion } from "../../lib/schema/flow/board-version";
import type { IElementDemand } from "../../lib/schema/flow/element-demand";
import {
	MAX_ELEMENTS_BYTES,
	materializeSurfaceElements,
} from "./element-materializer";
import type { SurfaceComponent } from "./types";
import {
	type WidgetElementScope,
	flattenSurfaceComponentsForElements,
	mergeStoredElementValues,
	withoutWidgetHosts,
} from "./workflow-elements";

export interface ElementDemandBoardState {
	getElementDemand?(
		appId: string,
		boardId: string,
		version?: BoardVersion,
	): Promise<IElementDemand>;
}

export type RunElementDemand = Pick<IElementDemand, "selectors">;

export interface CollectRunElementsInput {
	backend: { boardState: ElementDemandBoardState };
	appId?: string;
	boardId?: string;
	boardVersion?: BoardVersion;
	/** A Page contract's demand (from its bootstrap), used instead of asking the Board endpoint. */
	demand?: RunElementDemand | null;
	surfaceId: string;
	components: Record<string, SurfaceComponent> | undefined;
	storedValues: Record<string, unknown>;
	widgetScope?: WidgetElementScope;
	triggeringComponentId?: string;
	/** Skip the cache: the flow graph changes without a signal in preview mode. */
	refresh?: boolean;
}

interface DemandEntry {
	demand?: IElementDemand;
	fetchedAt: number;
	inflight?: Promise<IElementDemand>;
}

const demandCache = new Map<string, DemandEntry>();
const REVALIDATE_AFTER_MS = 15_000;

function demandKey(
	appId: string,
	boardId: string,
	version: BoardVersion | undefined,
): string {
	return `${appId}:${boardId}:${version ? version.join(".") : "latest"}`;
}

/**
 * Stale-while-revalidate. A rejected fetch propagates when it was the answer
 * (first load or `refresh`); a failed background revalidation keeps the entry.
 */
async function resolveElementDemand(
	input: CollectRunElementsInput,
): Promise<RunElementDemand | undefined> {
	if (input.demand) return input.demand;
	const { backend, appId, boardId, boardVersion, refresh } = input;
	const { boardState } = backend;
	const getElementDemand = boardState.getElementDemand;
	if (!appId || !boardId || typeof getElementDemand !== "function") {
		return undefined;
	}

	const key = demandKey(appId, boardId, boardVersion);
	const entry = demandCache.get(key);
	if (entry?.inflight) return entry.inflight;

	const fetchDemand = (): Promise<IElementDemand> => {
		const inflight = getElementDemand
			.call(boardState, appId, boardId, boardVersion)
			.then((demand) => {
				demandCache.set(key, { demand, fetchedAt: Date.now() });
				return demand;
			})
			.catch((error) => {
				const current = demandCache.get(key);
				if (current?.inflight === inflight) {
					if (current.demand) {
						demandCache.set(key, { ...current, inflight: undefined });
					} else {
						demandCache.delete(key);
					}
				}
				throw error;
			});
		demandCache.set(key, {
			...entry,
			fetchedAt: entry?.fetchedAt ?? 0,
			inflight,
		});
		return inflight;
	};

	if (!entry?.demand || refresh) return fetchDemand();
	if (Date.now() - entry.fetchedAt > REVALIDATE_AFTER_MS) {
		fetchDemand().catch(() => undefined);
	}
	return entry.demand;
}

function triggerSelectors(input: CollectRunElementsInput): string[] {
	const { surfaceId, widgetScope, triggeringComponentId } = input;
	if (!triggeringComponentId) return [];
	// A widget run may be triggered by the instance's own child or by a host-level
	// component; unresolvable candidates contribute nothing.
	return widgetScope?.instanceId
		? [
				`${surfaceId}/${triggeringComponentId}`,
				`${widgetScope.instanceId}/${triggeringComponentId}`,
			]
		: [`${surfaceId}/${triggeringComponentId}`];
}

/**
 * A widget run's own scope is its instance. A page run's is the page without widget
 * instances: every runtime-built list row would otherwise add its host and a copy of its
 * definition.
 */
function ownScopeElements(
	input: CollectRunElementsInput,
): Record<string, unknown> {
	const { surfaceId, components, storedValues, widgetScope } = input;
	if (widgetScope) {
		return mergeStoredElementValues(
			flattenSurfaceComponentsForElements(components, surfaceId, widgetScope),
			storedValues,
			components,
			surfaceId,
			widgetScope,
		);
	}
	const pageComponents = withoutWidgetHosts(components);
	return {
		...mergeStoredElementValues(
			flattenSurfaceComponentsForElements(pageComponents, surfaceId),
			storedValues,
			pageComponents,
			surfaceId,
		),
		...materializeSurfaceElements(
			{ surfaceId, components, storedValues },
			triggerSelectors(input),
		),
	};
}

/**
 * The `_elements` map a run starts with: the elements its board reads statically plus
 * the element that triggered it.
 *
 * Without a demand, or when a broad selector (`type:`, `glob:`) selects more than an
 * invocation can carry, the run sends its own scope. It asks the live page for anything
 * else it reads (`_elements_mode: "demand"`).
 */
export async function collectRunElements(
	input: CollectRunElementsInput,
): Promise<Record<string, unknown>> {
	const { surfaceId, components, storedValues, widgetScope } = input;

	let demand: RunElementDemand | undefined;
	try {
		demand = await resolveElementDemand(input);
	} catch (error) {
		console.warn(
			"[A2UI] Failed to fetch element demand, sending the run's own scope:",
			error,
		);
	}
	if (!demand) return ownScopeElements(input);

	const selected = materializeSurfaceElements(
		{ surfaceId, components, storedValues },
		[...demand.selectors, ...triggerSelectors(input)],
		widgetScope,
	);
	const selectedBytes = JSON.stringify(selected).length;
	if (selectedBytes <= MAX_ELEMENTS_BYTES) return selected;

	const own = ownScopeElements(input);
	console.warn(
		`[A2UI] The element demand selects ${selectedBytes} bytes (limit ${MAX_ELEMENTS_BYTES}); sending the run's own scope instead`,
	);
	return JSON.stringify(own).length < selectedBytes ? own : selected;
}

export function resetRunElementDemandCache(): void {
	demandCache.clear();
}
