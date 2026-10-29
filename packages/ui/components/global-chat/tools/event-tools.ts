import { createId } from "@paralleldrive/cuid2";
import { EVENT_DEFINITIONS } from "../../../lib/event-definitions";
import { isSavePromptRefusal } from "../../../lib/event-save-prompts";
import type { IEvent } from "../../../lib/schema/flow/event";
import { IEventExecutionMode } from "../../../lib/schema/flow/event";
import { nowSystemTime } from "../../../lib/time/now";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import type { IBackendState } from "../../../state/backend-state";
import type { IEventUpsertResult } from "../../../state/backend-state/event-state";
import {
	pageEventPersistenceReset,
	resolveAppEventTarget,
	resolveAppEventType,
} from "../app-event-target";
import { isRunnableWorkflowEventEntry } from "../workflow-event-entries";
import { argBool, argObject, argString } from "./tool-arguments";

export interface AppEventToolOptions {
	assertActive(): void;
	referenceApp(appId: string): void;
	/** Only the build host supplies reserved IDs. Ordinary updates must name an existing Event. */
	reservedEventId?: string;
}

/** Register an exact persisted entry and report incomplete route provisioning explicitly. */
export async function upsertAppEvent(
	backend: Pick<IBackendState, "eventState" | "boardState" | "routeState"> &
		Partial<Pick<IBackendState, "appState">>,
	args: Record<string, unknown>,
	options: AppEventToolOptions,
): Promise<Record<string, unknown>> {
	const appId = argString(args, "app_id") || argString(args, "appId");
	if (!appId)
		return {
			status: "error",
			message: "upsert_event requires an app_id.",
		};
	const name = argString(args, "name").trim();
	if (!name)
		return {
			status: "error",
			message: "upsert_event requires a name.",
		};
	if (backend.appState && argBool(args, "active") !== false) {
		const app = await backend.appState.getAppAuthoritative(appId);
		if (app.status === "Inactive") {
			return {
				status: "error",
				code: "staged_app_activation_blocked",
				message:
					"This app is inactive. Use app_build promotion after verification; direct Event activation is blocked.",
			};
		}
	}
	const eventId =
		argString(args, "event_id") ||
		argString(args, "eventId") ||
		options.reservedEventId ||
		"";
	let existingEvent: IEvent | undefined;
	if (eventId) {
		try {
			existingEvent = options.reservedEventId
				? (await backend.eventState.getEventsAuthoritative(appId)).find(
						(event) => event.id === eventId,
					)
				: await backend.eventState.getEvent(appId, eventId);
		} catch (error) {
			return {
				status: "error",
				message: `Cannot update event '${eventId}': ${error instanceof Error ? error.message : String(error)}`,
			};
		}
	}

	const target = resolveAppEventTarget({
		requestedPageId: argString(args, "page_id") || argString(args, "pageId"),
		requestedBoardId: argString(args, "board_id") || argString(args, "boardId"),
		requestedNodeId: argString(args, "node_id") || argString(args, "nodeId"),
		existingPageId: existingEvent?.default_page_id,
		existingBoardId: existingEvent?.board_id,
		existingNodeId: existingEvent?.node_id,
	});
	if (!target.ok) {
		return {
			status: "error",
			message: target.message,
		};
	}
	const { pageId, boardId: eventBoardId, nodeId: eventNodeId } = target;
	// A page Event may retain its owning board as metadata, but never a workflow
	// entry node. Clearing a stale node also repairs previously misclassified page
	// Events the next time FlowPilot updates them.

	let entryNodeName: string | undefined;
	let entryConfig: (typeof EVENT_DEFINITIONS)[string] | undefined;
	let boardExecutionMode: string | undefined;
	if (eventBoardId && eventNodeId) {
		let eventBoard: Awaited<
			ReturnType<typeof backend.boardState.getBoardAuthoritative>
		>;
		try {
			eventBoard = options.reservedEventId
				? await backend.boardState.getBoardAuthoritative(appId, eventBoardId)
				: await backend.boardState.getBoard(
						appId,
						eventBoardId,
						undefined,
						true,
					);
		} catch (error) {
			return {
				status: "error",
				message: `Failed to load the Event's board: ${error instanceof Error ? error.message : String(error)}`,
			};
		}
		const entryNode = eventBoard?.nodes?.[eventNodeId];
		entryNodeName = entryNode?.name;
		boardExecutionMode = eventBoard?.execution_mode;
		entryConfig = entryNodeName ? EVENT_DEFINITIONS[entryNodeName] : undefined;
		if (!entryNodeName || !entryConfig) {
			return {
				status: "error",
				message: `Node '${eventNodeId}' is not a supported Event entry. Use flowpilot_board to create eventsSimple(), eventsGeneric(payload: Struct, fieldName: string, ...), or eventsChat(...), then pass the returned event_nodes id.`,
			};
		}
		if (!isRunnableWorkflowEventEntry(eventBoard, eventNodeId)) {
			return {
				status: "error",
				message: `Node '${eventNodeId}' is an empty or unconnected Event entry. Build and connect the board logic first, then use the exact runnable event_nodes id returned by flowpilot_board. No Event was registered.`,
			};
		}
	}

	const requestedEventType = argString(args, "event_type").trim();
	const eventType = resolveAppEventType({
		pageId,
		requestedEventType,
		existingEventType: existingEvent?.event_type,
		supportedWorkflowEventTypes: entryConfig?.eventTypes,
		defaultWorkflowEventType: entryConfig?.defaultEventType,
	});
	if (entryConfig && !entryConfig.eventTypes.includes(eventType)) {
		return {
			status: "error",
			message: `Event type '${eventType}' is incompatible with ${entryNodeName}. Supported types: ${entryConfig.eventTypes.join(", ")}. Cron setup requires an events_simple entry.`,
		};
	}

	const requestedExecutionMode =
		argString(args, "execution_mode") || argString(args, "executionMode");
	let executionMode =
		requestedExecutionMode.toLowerCase() === "remote"
			? IEventExecutionMode.Remote
			: requestedExecutionMode.toLowerCase() === "local"
				? IEventExecutionMode.Local
				: (existingEvent?.execution_mode ?? IEventExecutionMode.Local);
	// Core enforces a concrete board mode on its Events. Resolve it here too so
	// sink_execution and the persisted Event cannot contradict one another.
	if (boardExecutionMode === "Local") executionMode = IEventExecutionMode.Local;
	if (boardExecutionMode === "Remote")
		executionMode = IEventExecutionMode.Remote;

	const existingConfig = pageId
		? undefined
		: parseUint8ArrayToJson(existingEvent?.config);
	const defaultConfig = entryConfig?.configs[eventType] ?? {};
	const keepExistingConfig =
		existingEvent?.event_type === eventType &&
		existingConfig &&
		typeof existingConfig === "object";
	let eventConfig: Record<string, unknown> = pageId
		? {}
		: {
				...(keepExistingConfig
					? (existingConfig as Record<string, unknown>)
					: (defaultConfig as Record<string, unknown>)),
				...(argObject(args, "config") ?? {}),
			};
	if (!pageId && eventType === "cron") {
		const expression =
			argString(args, "cron_expression") ||
			argString(args, "cronExpression") ||
			(typeof eventConfig.expression === "string"
				? eventConfig.expression.trim()
				: "");
		const scheduledFor =
			argObject(args, "scheduled_for") ||
			argObject(args, "scheduledFor") ||
			(eventConfig.scheduled_for &&
			typeof eventConfig.scheduled_for === "object"
				? (eventConfig.scheduled_for as Record<string, unknown>)
				: undefined);
		if (!expression && !scheduledFor) {
			return {
				status: "error",
				message:
					"A cron Event requires cron_expression for a recurring schedule OR scheduled_for {date, time} for a one-time run.",
			};
		}
		if (
			scheduledFor &&
			(typeof scheduledFor.date !== "string" ||
				typeof scheduledFor.time !== "string")
		) {
			return {
				status: "error",
				message:
					"scheduled_for requires string fields date (YYYY-MM-DD) and time (HH:mm).",
			};
		}
		const timezone =
			argString(args, "timezone") ||
			(typeof eventConfig.timezone === "string" ? eventConfig.timezone : "UTC");
		eventConfig = {
			...eventConfig,
			sink_type: "cron",
			timezone,
			last_fired: null,
			sink_execution:
				executionMode === IEventExecutionMode.Remote ? "REMOTE" : "LOCAL",
		};
		if (expression) {
			eventConfig.expression = expression;
			eventConfig.scheduled_for = undefined;
		} else {
			eventConfig.scheduled_for = scheduledFor;
			eventConfig.expression = undefined;
		}
	}

	const now = nowSystemTime();
	const pagePersistenceReset = pageEventPersistenceReset(pageId);
	const event: IEvent = {
		...(existingEvent ?? {}),
		id: eventId || createId(),
		name,
		description:
			argString(args, "description") || existingEvent?.description || "",
		board_id: eventBoardId,
		node_id: pagePersistenceReset?.nodeId ?? eventNodeId,
		config:
			pagePersistenceReset?.config ??
			convertJsonToUint8Array(eventConfig) ??
			[],
		inputs: pagePersistenceReset?.inputs ?? existingEvent?.inputs,
		canary: pagePersistenceReset ? null : existingEvent?.canary,
		board_version:
			target.kind === "page" && !target.preserveExistingPageMetadata
				? undefined
				: existingEvent?.board_version,
		active: argBool(args, "active") ?? existingEvent?.active ?? true,
		event_type: eventType,
		event_version: existingEvent?.event_version ?? [0, 0, 0],
		priority: existingEvent?.priority ?? 0,
		variables: existingEvent?.variables ?? {},
		created_at: existingEvent?.created_at ?? now,
		updated_at: now,
		execution_mode: executionMode,
		...(pageId ? { default_page_id: pageId } : {}),
	};
	let savedEvent: IEventUpsertResult;
	let writeAttempted = false;
	try {
		options.assertActive();
		writeAttempted = true;
		savedEvent = await backend.eventState.upsertEvent(appId, event);
	} catch (error) {
		const outcomeUnknown = writeAttempted && !isSavePromptRefusal(error);
		return {
			status: outcomeUnknown ? "unknown" : "error",
			code: outcomeUnknown
				? "event_write_outcome_unknown"
				: "event_write_cancelled",
			event_id: event.id,
			provisioning: {
				event_saved: outcomeUnknown ? "unknown" : false,
				route_applied: false,
			},
			message: `Failed to upsert event: ${error instanceof Error ? error.message : String(error)}`,
			...(outcomeUnknown
				? {
						recovery:
							"Inspect this exact Event ID before retrying. A missing response does not prove the Event was not created; do not create a replacement.",
					}
				: {}),
		};
	}
	// Optional URL route mapping (path -> eventId) so the event is reachable.
	const rawRoute = argString(args, "route");
	let routePath: string | undefined;
	let routeError: string | undefined;
	if (rawRoute) {
		routePath = rawRoute.startsWith("/") ? rawRoute : `/${rawRoute}`;
		try {
			options.assertActive();
			await backend.routeState.setRoute(appId, routePath, savedEvent.id);
		} catch (error) {
			routeError = error instanceof Error ? error.message : String(error);
		}
	}
	options.referenceApp(appId);
	const triggerError = savedEvent.trigger_error;
	return {
		status: routeError ? "partial" : triggerError ? "saved" : "ok",
		provisioning: {
			event_saved: true,
			route_applied: routePath ? !routeError : null,
		},
		...(routeError
			? { code: "event_route_incomplete", message: routeError }
			: {}),
		...(triggerError ? { trigger_error: triggerError } : {}),
		event_id: savedEvent.id,
		event_type: savedEvent.event_type,
		...(entryNodeName ? { entry_node_type: entryNodeName } : {}),
		execution_mode: savedEvent.execution_mode,
		...(pageId ? { page_id: pageId } : {}),
		...(routePath ? { route: routePath } : {}),
		note: pageId
			? "Page event upserted (bound to the page)."
			: eventType === "cron"
				? "Cron setup attached to the Simple Event entry."
				: "Compatible Event setup attached to the workflow entry.",
	};
}
