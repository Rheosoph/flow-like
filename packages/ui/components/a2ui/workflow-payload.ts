import { APP_QUERY_PARAM, appQueryContext } from "../../lib/app-route-url";
import { getFrontendStateStore } from "./frontend-state";

/**
 * Frontend context shared by every board run triggered from a surface: the
 * a2ui navigation nodes read `_route`/`_query_params` from the run payload and
 * the state nodes read `_page_id`/`_global_state`/`_page_state`. Widget
 * callbacks run the same nodes as plain workflow events, so every trigger path
 * must ship this block so state and navigation nodes receive their context.
 */
export function buildFrontendContextPayload(
	pageId: string | null | undefined,
	globalState: Record<string, unknown> | undefined,
	pageState: Record<string, unknown> | undefined,
): Record<string, unknown> {
	let queryContext: ReturnType<typeof appQueryContext> = { _query_params: {} };
	let route = "";
	if (typeof window !== "undefined") {
		const searchParams = new URLSearchParams(window.location.search);
		queryContext = appQueryContext(window.location.search);
		route = searchParams.has(APP_QUERY_PARAM)
			? (searchParams.get("route") ?? "/")
			: window.location.pathname;
	}

	return {
		_route: route,
		...queryContext,
		_page_id: pageId || "default",
		_global_state: globalState ?? {},
		_page_state: pageState ?? {},
	};
}

/** Read the latest state after hydration, including writes from earlier actions. */
export async function buildWorkflowFrontendContext(
	appId: string | undefined,
	pageId: string | null | undefined,
): Promise<Record<string, unknown>> {
	const stateId = pageId || "default";
	const store = getFrontendStateStore(appId);
	await store.ensureLoaded(stateId);
	const snapshot = store.getSnapshot();
	return buildFrontendContextPayload(
		stateId,
		snapshot.globalState,
		snapshot.pageStates[stateId],
	);
}

/**
 * Remove transient local upload previews without changing JSON semantics.
 * Undefined object fields are omitted (as JSON.stringify does), while undefined
 * array slots become null so indexes remain stable.
 */
export function compactWorkflowPayload(value: unknown): unknown {
	if (value === undefined) return undefined;
	if (value === null) return null;

	if (Array.isArray(value)) {
		return value.map((item) => compactWorkflowPayload(item) ?? null);
	}

	if (typeof value === "object") {
		const source = value as Record<string, unknown>;
		const isUploadValue =
			typeof source.name === "string" &&
			typeof source.size === "number" &&
			typeof source.type === "string";
		const compacted: Record<string, unknown> = {};

		for (const [key, childValue] of Object.entries(source)) {
			if (key === "dataUrl" && isUploadValue) continue;
			const nextValue = compactWorkflowPayload(childValue);
			if (nextValue !== undefined) compacted[key] = nextValue;
		}

		return compacted;
	}

	return value;
}
