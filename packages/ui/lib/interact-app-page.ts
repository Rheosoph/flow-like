import type {
	LivePageHandle,
	LivePageRunRecord,
} from "../components/a2ui/live-page-registry";
import { waitForLivePage } from "../components/a2ui/live-page-registry";
import type { IHelperState } from "../state/backend-state/helper-state";
import {
	INLINE_PAGE_REVEAL_EVENT,
	captureInlineAppPageSnapshots,
	uploadPageSnapshots,
} from "./app-page-snapshot";

/**
 * Shared executor for the interact_app_page FlowPilot tool: drive a live, rendered app page
 * (set input values, fire component events), await the workflow runs those events start, and
 * report the resulting page state — elements, run outcomes, and fresh screenshots.
 *
 * Used by both the global tool bridge (which can mount the page inline first) and the
 * board-panel runtime tool executor (which requires an already-visible page).
 */

export interface InteractAppPageAction {
	action: "set_value" | "trigger";
	component_id: string;
	value?: unknown;
	event?: string;
}

export interface InteractAppPageRequest {
	appId: string;
	eventId?: string;
	pageId?: string;
	actions: InteractAppPageAction[];
	captureScreenshots?: boolean;
	/** How long to wait for a live page handle before giving up. */
	waitForPageMs?: number;
}

const MAX_ELEMENT_ENTRIES = 150;
const MAX_VALUE_CHARS = 300;
const SETTLE_TIMEOUT_MS = 20_000;

/** Normalize the model-supplied actions array; entries without a component_id survive so the result can name the rejection. */
export function parseInteractActions(raw: unknown): InteractAppPageAction[] {
	if (!Array.isArray(raw)) return [];
	return raw
		.filter(
			(entry): entry is Record<string, unknown> =>
				typeof entry === "object" && entry !== null,
		)
		.map((entry) => ({
			action: entry.action === "trigger" ? "trigger" : "set_value",
			component_id:
				typeof entry.component_id === "string"
					? entry.component_id
					: typeof entry.componentId === "string"
						? entry.componentId
						: "",
			value: entry.value,
			event: typeof entry.event === "string" ? entry.event : undefined,
		}));
}

function compactValue(value: unknown): unknown {
	if (value === undefined || value === null) return value;
	if (typeof value === "string")
		return value.length > MAX_VALUE_CHARS
			? `${value.slice(0, MAX_VALUE_CHARS)}…`
			: value;
	if (typeof value === "number" || typeof value === "boolean") return value;
	try {
		const serialized = JSON.stringify(value);
		return serialized.length > MAX_VALUE_CHARS
			? `${serialized.slice(0, MAX_VALUE_CHARS)}…`
			: JSON.parse(serialized);
	} catch {
		return String(value);
	}
}

function compactRun(record: LivePageRunRecord) {
	return {
		status: record.status,
		run_id: record.runId,
		component_id: record.componentId,
		node_id: record.nodeId,
		board_id: record.boardId,
		...(record.errorMessage ? { error_message: record.errorMessage } : {}),
		...(record.logMeta
			? {
					log_level: record.logMeta.log_level,
					log_count: record.logMeta.logs ?? undefined,
				}
			: {}),
	};
}

function serializeElements(handle: LivePageHandle) {
	const surface = handle.getSurface();
	if (!surface) return { elements: [], truncated: false };
	const storedValues = handle.getElementValues();
	const entries = Object.entries(surface.components ?? {});
	const elements = entries
		.slice(0, MAX_ELEMENT_ENTRIES)
		.map(([componentId, surfaceComponent]) => {
			const component = surfaceComponent.component as
				| {
						type?: string;
						eventHandlers?: Record<string, unknown>;
				  }
				| undefined;
			const storedValue = storedValues[`${surface.id}/${componentId}`];
			return {
				component_id: componentId,
				type: component?.type ?? "unknown",
				...(storedValue !== undefined
					? { current_value: compactValue(storedValue) }
					: {}),
				configured_events: Object.keys(component?.eventHandlers ?? {}),
			};
		});
	return { elements, truncated: entries.length > MAX_ELEMENT_ENTRIES };
}

async function waitForSettled(handle: LivePageHandle): Promise<void> {
	const deadline = Date.now() + SETTLE_TIMEOUT_MS;
	while (handle.isLoading() && Date.now() < deadline) {
		await new Promise((resolve) => setTimeout(resolve, 150));
	}
	// One extra beat so streamed a2ui updates land in the surface before serialization.
	await new Promise((resolve) => setTimeout(resolve, 400));
}

export async function interactWithAppPage(
	backend: { helperState: IHelperState },
	request: InteractAppPageRequest,
): Promise<Record<string, unknown>> {
	const { appId, eventId, pageId } = request;
	// An inline page card mounts collapsed and only renders (and registers) its page once the
	// reveal event expands it — keep dispatching while waiting, mirroring the capture loop.
	const revealInterval = eventId
		? setInterval(() => {
				window.dispatchEvent(
					new CustomEvent(INLINE_PAGE_REVEAL_EVENT, {
						detail: { appId, eventId },
					}),
				);
			}, 150)
		: undefined;
	const handle = await waitForLivePage(
		appId,
		{ eventId, pageId },
		request.waitForPageMs ?? 15_000,
	).finally(() => {
		if (revealInterval) clearInterval(revealInterval);
	});
	if (!handle) {
		return {
			status: "error",
			code: "page_not_live",
			message: `No live rendered page for app '${appId}'${eventId ? ` event '${eventId}'` : ""}${pageId ? ` page '${pageId}'` : ""} is currently mounted. Embed the page first (open_app_page) or ask the user to open it, then retry.`,
		};
	}

	const appliedActions: Record<string, unknown>[] = [];
	const runs: ReturnType<typeof compactRun>[] = [];
	for (const action of request.actions) {
		if (!action.component_id) {
			appliedActions.push({
				action: action.action,
				ok: false,
				detail: "component_id is required.",
			});
			continue;
		}
		try {
			if (action.action === "set_value") {
				handle.setElementValue(action.component_id, action.value);
				appliedActions.push({
					action: "set_value",
					component_id: action.component_id,
					ok: true,
				});
			} else if (action.action === "trigger") {
				const eventName = action.event || "click";
				const result = await handle.triggerComponentEvent(
					action.component_id,
					eventName,
				);
				runs.push(...result.runs.map(compactRun));
				appliedActions.push({
					action: "trigger",
					component_id: action.component_id,
					event: eventName,
					ok: result.triggered,
					...(result.triggered
						? { handler_source: result.source, runs: result.runs.length }
						: {
								detail: `Component has no handler for '${eventName}' (no exact, wildcard, or legacy action applies). Check configured_events in the returned elements.`,
							}),
				});
			} else {
				appliedActions.push({
					action: String(action.action),
					component_id: action.component_id,
					ok: false,
					detail: "Unsupported action; use 'set_value' or 'trigger'.",
				});
			}
		} catch (error) {
			appliedActions.push({
				action: action.action,
				component_id: action.component_id,
				ok: false,
				detail: error instanceof Error ? error.message : String(error),
			});
		}
	}

	await waitForSettled(handle);
	const { elements, truncated } = serializeElements(handle);

	let screenshotFields: Record<string, unknown> = {
		screenshot_count: 0,
		screenshot_complete: false,
	};
	if (request.captureScreenshots !== false) {
		const snapshot = await captureInlineAppPageSnapshots(
			appId,
			handle.eventId ?? eventId,
			10_000,
		);
		const { uploaded, uploadErrors } = await uploadPageSnapshots(
			backend,
			snapshot.images,
		);
		screenshotFields = {
			screenshot_count: uploaded.length,
			screenshot_complete:
				snapshot.complete && uploaded.length === snapshot.images.length,
			...(uploadErrors.length > 0
				? { upload_errors: uploadErrors.slice(0, 3) }
				: {}),
			...(snapshot.failureReason
				? { capture_failure: snapshot.failureReason }
				: {}),
			...(uploaded.length > 0 ? { _flowpilot_image_urls: uploaded } : {}),
		};
	}

	const failedActions = appliedActions.filter((entry) => entry.ok === false);
	const failedRuns = runs.filter((run) => run.status === "error");
	return {
		status: failedActions.length === 0 ? "ok" : "partial",
		app_id: appId,
		...((handle.eventId ?? eventId)
			? { event_id: handle.eventId ?? eventId }
			: {}),
		page_id: handle.pageId,
		applied_actions: appliedActions,
		runs,
		run_count: runs.length,
		...(failedRuns.length > 0 ? { failed_run_count: failedRuns.length } : {}),
		elements,
		...(truncated ? { elements_truncated: true } : {}),
		...screenshotFields,
		note: `Inspect 'runs' for the workflow executions your triggers started (use query_execution_logs with a run_id for full logs), 'elements' for the page's post-run input state, and the attached screenshots for the rendered result.`,
	};
}
