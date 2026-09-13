import { classifyAppEventInterface } from "@flow-like/flow-like-ui/components/global-chat/app-event-interface";
import {
	callMcpAppTool,
	describeMcpAppInterface,
} from "@flow-like/flow-like-ui/components/global-chat/app-mcp-interface";
import {
	APP_QUERY_PARAM,
	type AppQueryParam,
	appRouteUrl,
	parseAppRouteTarget,
	readAppQuery,
} from "@flow-like/flow-like-ui/lib/app-route-url";
import type { ActiveEventExecution } from "@flow-like/flow-like-ui/lib/execution-engine";
import {
	type NativeSurface,
	isNativeEventExposed,
	nativeEventActionKind,
	nativeEventSettings,
} from "@flow-like/flow-like-ui/lib/native-event";
import type { RecentAppUse } from "@flow-like/flow-like-ui/lib/recent-apps";
import { routePathsEqual } from "@flow-like/flow-like-ui/lib/route-path";
import {
	BUILTIN_RUNTIME_EVENT_TYPE_SET,
	deriveRouteMappings,
	isUsableRuntimeEvent,
} from "@flow-like/flow-like-ui/lib/runtime-route";
import type { IEvent } from "@flow-like/flow-like-ui/lib/schema/flow/event";
import type { IExecutionUsageRecord } from "@flow-like/flow-like-ui/lib/schema/usage/tracking";
import { parseUint8ArrayToJson } from "@flow-like/flow-like-ui/lib/uint8";
import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";

export type NativeActionKind =
	| "open_home"
	| "open_inbox"
	| "open_app"
	| "open_event"
	| "run_event"
	| "open_run"
	| "flowpilot"
	| "share";
export interface NativeAction {
	path?: string;
	queryParams?: AppQueryParam[];
	kind: NativeActionKind;
	appId?: string;
	eventId?: string;
	runId?: string;
	prompt?: string;
	voice?: boolean;
	operation?: string;
	text?: string;
	files?: string[];
}
export interface NativePendingAction {
	id: string;
	scope: string;
	action: NativeAction;
	responseMode?: "text" | "result";
	responseDeadline?: string;
}
import type {
	NativeNotificationIcon,
	NativeNotificationIconSource,
} from "./native-notification-icons";
export interface NativeSnapshotItem {
	icon?: NativeNotificationIcon;
	id: string;
	title: string;
	subtitle?: string;
	value?: string;
	status?: string;
	progress?: number;
	startedAt?: string;
	action: NativeAction;
}
export type NativeWidgetKind =
	| "flowpilot"
	| "inbox"
	| "attention"
	| "recent_runs"
	| "recent_apps"
	| "usage"
	| "workspace"
	| "event_favorites";
export interface NativeSnapshotSection {
	kind: NativeWidgetKind;
	title: string;
	state: "ready" | "unavailable" | "signed_out";
	items: NativeSnapshotItem[];
}
export interface NativeEventEntity {
	id: string;
	appId: string;
	eventId: string;
	title: string;
	subtitle: string;
	eventType: string;
	route?: string;
	pageId?: string;
	action: NativeAction;
	surfaces: NativeSurface[];
}
export interface NativeSnapshot {
	version: 1;
	scope: string;
	generatedAt: string;
	expiresAt: string;
	sections: NativeSnapshotSection[];
	events: NativeEventEntity[];
	apps: { id: string; title: string; spotlightEligible?: boolean }[];
	activePage?: { title: string; url: string };
	webOrigin?: string;
}

const settledValue = <T>(result: PromiseSettledResult<T>): T | undefined =>
	result.status === "fulfilled" ? result.value : undefined;
const section = (
	kind: NativeWidgetKind,
	title: string,
	items: NativeSnapshotItem[],
	available = true,
): NativeSnapshotSection => ({
	kind,
	title,
	items,
	state: available ? "ready" : "unavailable",
});

/** Copies only display data into the native cache. Tokens, config and input defaults stay in-app. */
export async function loadNativeSnapshot(
	backend: IBackendState,
	scope: string,
	recent: RecentAppUse[],
	authenticated: boolean,
	now = new Date(),
	onNotificationSources?: (sources: NativeNotificationIconSource[]) => void,
): Promise<NativeSnapshot> {
	const [
		libraryResult,
		profileResult,
		inboxResult,
		historyResult,
		activityResult,
		usageResult,
	] = await Promise.allSettled([
		backend.appState.getApps(),
		backend.userState.getProfile(),
		authenticated
			? backend.userState.listNotifications(true, undefined, 0, 8)
			: Promise.resolve(undefined),
		authenticated && backend.usageState
			? backend.usageState.getExecutionHistory(0, 8)
			: Promise.resolve(undefined),
		authenticated && backend.usageState
			? backend.usageState.getExecutionActivity(7)
			: Promise.resolve(undefined),
		authenticated && backend.usageState
			? backend.usageState.getUsageSummary()
			: Promise.resolve(undefined),
	]);
	const profile = settledValue(profileResult);
	const visible = new Set(profile?.apps?.map((app) => app.app_id) ?? []);
	let library = (settledValue(libraryResult) ?? []).filter(([app]) =>
		visible.has(app.id),
	);
	if (!authenticated) {
		const local = await Promise.allSettled(
			library.map(
				([app]) => backend.isLocalOnly?.(app.id) ?? Promise.resolve(false),
			),
		);
		library = library.filter(
			(_, index) =>
				local[index].status === "fulfilled" && local[index].value === true,
		);
	}
	const apps = library.map(([app, meta]) => ({
		id: app.id,
		title: meta?.name || app.id,
		spotlightEligible: false,
	}));
	const names = new Map(apps.map((app) => [app.id, app.title]));
	const events: NativeEventEntity[] = [];
	const favorites: NativeSnapshotItem[] = [];
	let eventReadsSucceeded =
		libraryResult.status === "fulfilled" &&
		profileResult.status === "fulfilled";
	// Bound native catalog discovery; one failed app does not hide other apps.
	for (let index = 0; index < library.length; index += 4) {
		const batch = library.slice(index, index + 4);
		const results = await Promise.allSettled(
			batch.map(([app]) => backend.eventState.getEvents(app.id)),
		);
		results.forEach((result, offset) => {
			if (result.status === "rejected") {
				eventReadsSucceeded = false;
				return;
			}
			const appId = batch[offset][0].id;
			const app = apps.find((app) => app.id === appId);
			if (app)
				app.spotlightEligible = result.value.some(
					(event) =>
						event.active &&
						["page", "chat"].includes(classifyAppEventInterface(event)),
				);
			for (const event of result.value) {
				const kind = nativeEventActionKind(event);
				if (!kind || !isNativeEventExposed(event)) continue;
				const settings = nativeEventSettings(event);
				const operation =
					event.event_type === "mcp" ? settings.operation : undefined;
				const action: NativeAction = {
					kind,
					appId,
					eventId: event.id,
					operation,
				};
				const entity: NativeEventEntity = {
					id: `${appId}:${event.id}${operation ? `:${operation}` : ""}`,
					appId,
					eventId: event.id,
					title: event.name || event.id,
					subtitle: names.get(appId) || appId,
					eventType: event.event_type,
					route: event.route || (event.is_default ? "/" : undefined),
					pageId: event.default_page_id || undefined,
					action,
					surfaces: settings.surfaces,
				};
				events.push(entity);
				if (settings.favorite && settings.surfaces.includes("widget"))
					favorites.push({
						id: entity.id,
						title: entity.title,
						subtitle: entity.subtitle,
						action,
					});
			}
		});
	}
	const runItem = (run: IExecutionUsageRecord): NativeSnapshotItem => ({
		id: run.id,
		title: names.get(run.app_id || "") || "Workflow run",
		subtitle: run.status,
		status: run.status,
		startedAt: run.created_at,
		action: { kind: "open_run", appId: run.app_id || undefined, runId: run.id },
	});
	const visibleRun = (run: IExecutionUsageRecord) =>
		Boolean(run.app_id && names.has(run.app_id));
	const inbox = settledValue(inboxResult);
	const appIcons = new Map(library.map(([app, meta]) => [app.id, meta?.icon]));
	onNotificationSources?.(
		(inbox ?? []).slice(0, 8).map((item) => ({
			id: item.id,
			icon: item.icon,
			appIcon: item.app_id ? appIcons.get(item.app_id) : undefined,
		})),
	);
	const history = settledValue(historyResult);
	const activity = settledValue(activityResult);
	const usage = settledValue(usageResult);
	const sections = [
		section("flowpilot", "FlowPilot", [
			{
				id: "flowpilot",
				title: "Ask FlowPilot",
				action: { kind: "flowpilot" },
			},
		]),
		section(
			"inbox",
			"Inbox",
			(inbox ?? []).map((item) => ({
				id: item.id,
				title: item.title,
				subtitle: item.description || undefined,
				action: { kind: "open_inbox", appId: item.app_id || undefined },
			})),
			inbox !== undefined,
		),
		section(
			"attention",
			"Needs attention",
			(activity?.attention ?? []).filter(visibleRun).slice(0, 8).map(runItem),
			activity !== undefined,
		),
		section(
			"recent_runs",
			"Recent runs",
			(history?.items ?? []).filter(visibleRun).map(runItem),
			history !== undefined,
		),
		section(
			"recent_apps",
			"Recently used apps",
			recent
				.filter((row) => names.has(row.appId))
				.slice(0, 8)
				.map((row) => ({
					id: row.appId,
					title: names.get(row.appId) ?? row.appId,
					subtitle: row.lastOpenedAt,
					action: { kind: "open_app", appId: row.appId },
				})),
			libraryResult.status === "fulfilled" &&
				profileResult.status === "fulfilled",
		),
		section(
			"usage",
			"Usage",
			usage
				? [
						{
							id: "executions",
							title: "Executions",
							value: String(usage.total_executions),
							action: { kind: "open_home" },
						},
						{
							id: "model-usage",
							title: "Model calls",
							value: String(usage.total_llm_invocations),
							action: { kind: "open_home" },
						},
						{
							id: "cost",
							title: "Recorded AI cost",
							value: `$${((usage.total_llm_price + usage.total_embedding_price) / 1_000_000).toFixed(2)}`,
							subtitle: "USD · all recorded usage",
							action: { kind: "open_home" },
						},
					]
				: [],
			usage !== undefined,
		),
		section(
			"workspace",
			"Workspace",
			[
				{
					id: "apps",
					title: "Apps",
					value: String(apps.length),
					action: { kind: "open_home" },
				},
				...(activity
					? [
							{
								id: "activity",
								title: "Account executions · last 7 days",
								value: String(activity.total),
								action: { kind: "open_home" as const },
							},
						]
					: []),
			],
			libraryResult.status === "fulfilled" &&
				profileResult.status === "fulfilled",
		),
		section(
			"event_favorites",
			"Favorite Events",
			favorites.slice(0, 12),
			eventReadsSucceeded,
		),
	];
	if (!authenticated)
		for (const item of sections)
			if (["inbox", "attention", "recent_runs", "usage"].includes(item.kind))
				item.state = "signed_out";
	return {
		version: 1,
		scope,
		generatedAt: now.toISOString(),
		expiresAt: new Date(now.getTime() + 60 * 60 * 1000).toISOString(),
		sections,
		events,
		apps,
	};
}

export function nativeEventUrl(
	appId: string,
	event: IEvent,
	prompt?: string,
): string {
	const query = new URLSearchParams({ id: appId, eventId: event.id });
	if (prompt) query.set("message", prompt);
	return `/use?${query}`;
}

/** Missing required data goes through the existing form, with its input and consent UI. */
export function nativeQuickActionPayload(
	event: IEvent,
	text?: string,
): Record<string, unknown> | null {
	const inputs = (event.inputs ?? []).filter(
		(input) => input.data_type !== "Execution",
	);
	let supplied: Record<string, unknown> = {};
	if (text?.trim()) {
		let parsed: unknown;
		try {
			parsed = JSON.parse(text);
		} catch {
			parsed = text;
		}
		if (parsed && typeof parsed === "object" && !Array.isArray(parsed))
			supplied = parsed as Record<string, unknown>;
		else if (
			typeof parsed === "string" &&
			inputs.length === 1 &&
			inputs[0].data_type === "String" &&
			(!inputs[0].value_type || inputs[0].value_type === "Normal")
		)
			supplied = { [inputs[0].name]: parsed };
		else
			throw new Error(
				"Shortcut input must be a JSON object keyed by Event input names, or text for a single String input.",
			);
		for (const name of Object.keys(supplied))
			if (!inputs.some((input) => input.name === name))
				throw new Error(`This Event has no input named ${name}.`);
	}
	const entries: [string, unknown][] = [];
	for (const input of inputs) {
		if (Object.hasOwn(supplied, input.name)) {
			const value = supplied[input.name];
			if (!nativeInputMatches(input, value))
				throw new Error(
					`Input ${input.name} must match its ${input.value_type || "Normal"} ${input.data_type} type.`,
				);
			entries.push([input.name, value]);
			continue;
		}
		if (input.default_value?.length) {
			try {
				const parsed = parseUint8ArrayToJson(input.default_value);
				if (parsed == null) return null;
				entries.push([input.name, parsed]);
			} catch {
				return null;
			}
		} else if (!input.optional) return null;
	}
	return Object.fromEntries(entries);
}

function nativeInputMatches(
	input: NonNullable<IEvent["inputs"]>[number],
	value: unknown,
): boolean {
	if (value == null) return input.optional === true;
	const scalar = (item: unknown): boolean => {
		switch (input.data_type) {
			case "String":
			case "PathBuf":
				return typeof item === "string";
			case "Boolean":
				return typeof item === "boolean";
			case "Integer":
				return typeof item === "number" && Number.isSafeInteger(item);
			case "Byte":
				return (
					typeof item === "number" &&
					Number.isInteger(item) &&
					item >= 0 &&
					item <= 255
				);
			case "Float":
				return typeof item === "number" && Number.isFinite(item);
			case "Date":
				return typeof item === "string" && Number.isFinite(Date.parse(item));
			case "Struct":
			case "Geometry":
				return !!item && typeof item === "object" && !Array.isArray(item);
			case "Generic":
				return item !== undefined;
			default:
				return false;
		}
	};
	if (input.value_type === "Array" || input.value_type === "HashSet")
		return Array.isArray(value) && value.every(scalar);
	if (input.value_type === "HashMap")
		return (
			typeof value === "object" &&
			!Array.isArray(value) &&
			Object.values(value).every(scalar)
		);
	return scalar(value);
}

export interface NativeDispatchContext {
	scope: string;
	backend: IBackendState;
	navigate: (url: string) => void;
	flowpilot: (
		prompt?: string,
		voice?: boolean,
		files?: string[],
	) => Promise<void> | void;
	execute: (
		appId: string,
		event: IEvent,
		payload: Record<string, unknown>,
		requestId: string,
	) => Promise<unknown>;
	callEvent?: (
		appId: string,
		event: IEvent,
		input?: string,
	) => Promise<void> | void;
	mcp?: (
		appId: string,
		event: IEvent,
		tool: NativeMcpTool,
		requestId: string,
		input?: string,
	) => Promise<void> | void;
	isCurrent: () => boolean;
}

export async function dispatchNativeAction(
	request: NativePendingAction,
	context: NativeDispatchContext,
): Promise<void> {
	if (!request.id || request.scope !== context.scope || !context.isCurrent())
		throw new Error("This action belongs to a different account or workspace.");
	const deadline = request.responseDeadline
		? Date.parse(request.responseDeadline)
		: undefined;
	if (
		deadline !== undefined &&
		(!Number.isFinite(deadline) || deadline <= Date.now())
	)
		throw new Error(
			"This Shortcut request expired. Run it again to get a response.",
		);
	const action = request.action;
	if (action.kind === "open_home") {
		context.navigate("/");
		return;
	}
	if (action.kind === "open_inbox") {
		context.navigate("/notifications");
		return;
	}
	if (action.kind === "flowpilot" || action.kind === "share") {
		await context.flowpilot(
			action.prompt || action.text,
			action.voice,
			action.files,
		);
		return;
	}
	if (!action.appId) throw new Error("This native action is missing its app.");
	await context.backend.appState.getApp(action.appId);
	if (!context.isCurrent()) return;
	if (action.kind === "open_app") {
		const target = parseAppRouteTarget(action.path, action.queryParams);
		const profile = await context.backend.userState.getProfile();
		if (!context.isCurrent()) return;
		if (!profile.apps?.some((app) => app.app_id === action.appId))
			throw new Error("This app is no longer in the selected workspace.");
		if (target.path) {
			let mapping =
				await context.backend.routeState.getRouteByPathAuthoritative(
					action.appId,
					target.path,
				);
			if (!context.isCurrent()) return;
			// Some native route stores compare raw paths and omit synthesized root routes.
			// Resolve the runtime's canonical candidate, then verify its fresh Event below.
			if (!mapping) {
				const events = await context.backend.eventState.getEvents(
					action.appId,
					true,
				);
				if (!context.isCurrent()) return;
				mapping =
					deriveRouteMappings(events).find((route) =>
						routePathsEqual(route.path, target.path),
					) ?? null;
			}
			if (!mapping || !routePathsEqual(mapping.path, target.path))
				throw new Error(
					"This path is no longer available in the selected app.",
				);
			const event = await context.backend.eventState.getEventAuthoritative(
				action.appId,
				mapping.eventId,
			);
			if (!context.isCurrent()) return;
			const currentPath = event.route || (event.is_default ? "/" : undefined);
			if (
				currentPath === undefined ||
				!routePathsEqual(currentPath, target.path) ||
				!isUsableRuntimeEvent(event, BUILTIN_RUNTIME_EVENT_TYPE_SET)
			)
				throw new Error("This app path no longer opens an active interface.");
		}
		context.navigate(appRouteUrl(action.appId, target));
		return;
	}
	if (action.kind === "open_run") {
		context.navigate(
			`/library/config/analytics?id=${encodeURIComponent(action.appId)}`,
		);
		return;
	}
	if (!action.eventId)
		throw new Error("This native action is missing its Event.");
	const event = await context.backend.eventState.getEventAuthoritative(
		action.appId,
		action.eventId,
	);
	if (!context.isCurrent()) return;
	if (!isNativeEventExposed(event))
		throw new Error(
			"This Event is no longer available to native integrations.",
		);
	if (request.responseMode) {
		if (action.operation && event.event_type !== "mcp")
			throw new Error("This Event no longer exposes that operation.");
		const profile = await context.backend.userState.getProfile();
		if (!context.isCurrent()) return;
		if (!profile.apps?.some((app) => app.app_id === action.appId))
			throw new Error("This app is no longer in the selected workspace.");
		if (
			!nativeEventSettings(event).surfaces.some(
				(surface) => surface === "siri" || surface === "shortcuts",
			)
		)
			throw new Error(
				"This Event is no longer available to Siri and Shortcuts.",
			);
		if (!context.callEvent)
			throw new Error(
				"This app version cannot return Event results to Shortcuts.",
			);
		await context.callEvent(action.appId, event, action.text || action.prompt);
		return;
	}
	if (event.event_type === "mcp") {
		if (action.kind !== "run_event" || !context.mcp)
			throw new Error("This MCP entry point cannot run here.");
		const tool = await resolveNativeMcpTool(
			context.backend,
			action.appId,
			event,
			action.operation,
		);
		if (context.isCurrent())
			await context.mcp(
				action.appId,
				event,
				tool,
				request.id,
				action.text || action.prompt,
			);
		return;
	}
	if (action.operation)
		throw new Error("This Event no longer exposes that operation.");
	const url = nativeEventUrl(action.appId, event, action.prompt || action.text);
	context.navigate(url);
	if (
		action.kind === "run_event" &&
		nativeEventActionKind(event) === "run_event"
	) {
		const payload = nativeQuickActionPayload(
			event,
			action.text || action.prompt,
		);
		if (payload !== null)
			await context.execute(action.appId, event, payload, request.id);
	}
}

export interface NativeMcpTool {
	name: string;
	description?: string;
	inputSchema?: Record<string, unknown>;
}

export async function resolveNativeMcpTool(
	backend: IBackendState,
	appId: string,
	event: IEvent,
	operation?: string,
): Promise<NativeMcpTool> {
	if (
		!isNativeEventExposed(event) ||
		event.event_type !== "mcp" ||
		!operation ||
		nativeEventSettings(event).operation !== operation
	)
		throw new Error(
			"This MCP operation is no longer available to native integrations.",
		);
	const inventory = await describeMcpAppInterface(
		backend.eventState,
		appId,
		event.id,
	);
	const tool = inventory.tools.find(
		(item: unknown) =>
			!!item &&
			typeof item === "object" &&
			(item as NativeMcpTool).name === operation,
	);
	if (!tool)
		throw new Error(
			"The selected MCP tool is no longer registered. Update this Event's native settings.",
		);
	return tool as NativeMcpTool;
}

/** Recheck saved exposure after the arguments form has been open, before making a tool call. */
export async function executeNativeMcpOperation(
	backend: IBackendState,
	appId: string,
	eventId: string,
	operation: string,
	args: unknown,
	isCurrent: () => boolean,
) {
	if (!args || typeof args !== "object" || Array.isArray(args))
		throw new Error("Tool arguments must be a JSON object.");
	if (!isCurrent()) throw new Error("The active account or workspace changed.");
	const event = await backend.eventState.getEventAuthoritative(appId, eventId);
	if (!isCurrent()) throw new Error("The active account or workspace changed.");
	await resolveNativeMcpTool(backend, appId, event, operation);
	if (!isCurrent()) throw new Error("The active account or workspace changed.");
	return callMcpAppTool(
		backend.eventState,
		appId,
		eventId,
		{
			mcp_tool: operation,
			payload: args,
		},
		() => {
			if (!isCurrent())
				throw new Error("The active account or workspace changed.");
		},
	);
}

/** Deep links can open surfaces; executing Events always comes from the native action queue. */
export function nativeNavigationRequest(
	raw: string,
	scope: string,
): NativePendingAction | undefined {
	let url: URL;
	try {
		url = new URL(raw);
	} catch {
		return;
	}
	if (
		url.protocol !== "flow-like:" ||
		url.hostname !== "native" ||
		url.username ||
		url.password ||
		url.port ||
		url.hash
	)
		return;
	if (url.pathname === "/home")
		return { id: crypto.randomUUID(), scope, action: { kind: "open_home" } };
	if (url.searchParams.get("scope") !== scope) return;
	if (url.pathname === "/flowpilot" || url.pathname === "/inbox")
		return {
			id: crypto.randomUUID(),
			scope,
			action: {
				kind: url.pathname === "/flowpilot" ? "flowpilot" : "open_inbox",
			},
		};
	const appId = url.searchParams.get("appId") || undefined;
	if (url.pathname === "/app") {
		if (!appId) return;
		try {
			const query = url.searchParams.get("queryParams");
			const target = parseAppRouteTarget(
				url.searchParams.get("path") ?? undefined,
				query === null ? undefined : JSON.parse(query),
			);
			return {
				id: crypto.randomUUID(),
				scope,
				action: { kind: "open_app", appId, ...target },
			};
		} catch {
			return;
		}
	}
	if (url.pathname === "/run")
		return {
			id: crypto.randomUUID(),
			scope,
			action: {
				kind: "open_run",
				appId,
				runId: url.searchParams.get("runId") || undefined,
			},
		};
	if (url.pathname === "/page")
		return {
			id: crypto.randomUUID(),
			scope,
			action: {
				kind: "open_event",
				appId,
				eventId: url.searchParams.get("eventId") || undefined,
			},
		};
}

export function withNativeActiveRuns(
	snapshot: NativeSnapshot,
	runs: ActiveEventExecution[],
): NativeSnapshot {
	const visible = new Set(snapshot.apps.map((app) => app.id));
	const active: NativeSnapshotItem[] = runs
		.filter(
			(run) =>
				visible.has(run.appId) && (!run.scope || run.scope === snapshot.scope),
		)
		.map((run) => ({
			id: run.runId || run.streamId,
			title: run.title,
			subtitle: "Running",
			status: "running",
			startedAt: run.startedAt,
			action: {
				kind: "open_run",
				appId: run.appId,
				eventId: run.eventId,
				runId: run.runId || run.streamId,
			},
		}));
	const ids = new Set(active.map((item) => item.id));
	return {
		...snapshot,
		sections: snapshot.sections.map((section) =>
			section.kind === "recent_runs"
				? {
						...section,
						state: active.length ? "ready" : section.state,
						items: [
							...active,
							...section.items.filter((item) => !ids.has(item.id)),
						].slice(0, 12),
					}
				: section,
		),
	};
}

export function nativeWebOrigin(hubApp: unknown): string | undefined {
	if (typeof hubApp !== "string" || !hubApp.trim()) return;
	try {
		const url = new URL(hubApp.includes("://") ? hubApp : `https://${hubApp}`);
		if (
			url.protocol !== "https:" ||
			url.username ||
			url.password ||
			url.search ||
			url.hash ||
			url.pathname !== "/"
		)
			return;
		return url.origin;
	} catch {
		return;
	}
}

export function withNativeActivePage(
	snapshot: NativeSnapshot,
	pathname: string,
	query: string,
	webOrigin?: string,
): NativeSnapshot {
	const next = {
		...snapshot,
		webOrigin,
		activePage: undefined as NativeSnapshot["activePage"],
	};
	if (
		!webOrigin ||
		nativeWebOrigin(webOrigin) !== webOrigin ||
		pathname !== "/use"
	)
		return next;
	const params = new URLSearchParams(query);
	const appId = params.get("id");
	if (!snapshot.apps.some((app) => app.id === appId)) return next;
	const route = params.get("route");
	const event = snapshot.events.find(
		(event) =>
			event.appId === appId &&
			(route !== null
				? event.route !== undefined && routePathsEqual(event.route, route)
				: event.eventId === params.get("eventId")) &&
			event.action.kind === "open_event",
	);
	if (event) {
		let href = nativeEventUrl(event.appId, { id: event.eventId } as IEvent);
		const appOwnedQuery = params.has(APP_QUERY_PARAM);
		const handoffRoute = route ?? (appOwnedQuery ? event.route : undefined);
		if (appOwnedQuery && handoffRoute === undefined) return next;
		if (handoffRoute !== undefined) {
			try {
				const queryParams = appOwnedQuery
					? Array.from(readAppQuery(query), ([name, value]) => ({
							name,
							value,
						}))
					: [];
				href = appRouteUrl(
					event.appId,
					parseAppRouteTarget(handoffRoute, queryParams),
				);
			} catch {
				return next;
			}
		}
		next.activePage = {
			title: event.title,
			url: new URL(href, webOrigin).href,
		};
	}
	return next;
}
