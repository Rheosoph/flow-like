import type { BoardVersion } from "../../lib/schema/flow/board-version";
import type { IElementDemand } from "../../lib/schema/flow/element-demand";
import { materializeSurfaceElements } from "./element-materializer";
import type { SurfaceComponent } from "./types";
import {
	type WidgetElementScope,
	flattenSurfaceComponentsForElements,
	mergeStoredElementValues,
} from "./workflow-elements";

export interface ElementDemandBoardState {
	getElementDemand?(
		appId: string,
		boardId: string,
		version?: BoardVersion,
	): Promise<IElementDemand>;
}

export interface CollectRunElementsInput {
	backend: { boardState: ElementDemandBoardState };
	appId?: string;
	boardId?: string;
	boardVersion?: BoardVersion;
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
): Promise<IElementDemand | undefined> {
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

/**
 * The `_elements` map a run starts with: the elements its board reads statically plus
 * the element that triggered it. Without a reachable demand the full surface is sent,
 * as before the manifest existed.
 */
export async function collectRunElements(
	input: CollectRunElementsInput,
): Promise<Record<string, unknown>> {
	const { surfaceId, components, storedValues, widgetScope } = input;

	let demand: IElementDemand | undefined;
	try {
		demand = await resolveElementDemand(input);
	} catch (error) {
		console.warn(
			"[A2UI] Failed to fetch element demand, sending every element:",
			error,
		);
	}

	if (!demand) {
		return mergeStoredElementValues(
			flattenSurfaceComponentsForElements(components, surfaceId, widgetScope),
			storedValues,
			components,
			surfaceId,
			widgetScope,
		);
	}

	const selectors = [...demand.selectors];
	if (input.triggeringComponentId) {
		// A widget run may be triggered by the instance's own child or by a host-level
		// component; unresolvable candidates contribute nothing.
		selectors.push(`${surfaceId}/${input.triggeringComponentId}`);
		if (widgetScope?.instanceId) {
			selectors.push(
				`${widgetScope.instanceId}/${input.triggeringComponentId}`,
			);
		}
	}
	return materializeSurfaceElements(
		{ surfaceId, components, storedValues },
		selectors,
		widgetScope,
	);
}

export function resetRunElementDemandCache(): void {
	demandCache.clear();
}
