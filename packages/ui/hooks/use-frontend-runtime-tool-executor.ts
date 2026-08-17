"use client";

import { useCallback } from "react";

import type { SurfaceComponent } from "../components/a2ui/types";
import { compactJson, compactLogEvents } from "../components/flowpilot/utils";
import type { ILog, ILogMetadata, IRunPayload } from "../lib";
import { ApiResponseError } from "../lib/api-error";
import { runAppChatMessage } from "../lib/app-chat-run";
import {
	getPendingDatabaseSchemas,
	isExplicitSchemaCreateUnavailable,
	markExplicitSchemaCreateUnavailable,
	retainPendingDatabaseSchema,
} from "../lib/database-capability-session";
import { normalizeDatabaseTableIdentifier } from "../lib/database-table-name";
import {
	interactWithAppPage,
	parseInteractActions,
} from "../lib/interact-app-page";
import {
	encodePackageWidgetRef,
	listAppPackageWidgets,
} from "../lib/package-widgets";
import { type IBackendState, useBackend } from "../state/backend-state";
import type { IBoardState } from "../state/backend-state/board-state";
import { IIndexType } from "../state/backend-state/db-state";
import type {
	IDatabaseSchemaField,
	IDatabaseState,
} from "../state/backend-state/db-state";
import type {
	CreateOverlayPayload,
	GraphOverlay,
	IGraphState,
	InvokeOntologyActionPayload,
	NeighborsPayload,
	OntologyObjectRef,
	SubgraphPayload,
	UpdateOverlayPayload,
} from "../state/backend-state/graph-state";
import type { IPage } from "../state/backend-state/page-state";
import type { IWidget } from "../state/backend-state/widget-state";
import { useExecutionServiceOptional } from "../state/execution-service-context";

type UiInspectWidgetMetadata = {
	name?: unknown;
	title?: unknown;
	description?: unknown;
};

type UiInspectWidgetListEntry = readonly [
	appId: string,
	widgetId: string,
	metadata?: UiInspectWidgetMetadata,
];

type UiInspectWidgetEntry = {
	widgetId: string;
	selector: string;
	description?: string;
};

function nonEmptyMetadataText(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	const trimmed = value.trim();
	return trimmed.length > 0 ? trimmed : undefined;
}

/**
 * A package-shipped widget as `ui_inspect` reports it. The model cannot invent any of these
 * fields — `microWidgetInstance` needs them verbatim, and desktop serves the bundle by
 * `bundle_hash` — so the summary carries everything needed to place one.
 */
type UiInspectPackageWidget = {
	selector: string;
	package_id: string;
	widget_id: string;
	package_version: string;
	bundle_hash?: string;
	name: string;
	description?: string;
	contract: unknown;
};

/**
 * Package widgets of the app, resolved from installed manifests. Returns an empty list on hosts
 * without per-app package listing (web) and never throws: a widget-source failure must not take
 * down the page/widget inspection the model actually asked for.
 */
async function loadUiInspectPackageWidgets(
	backend: IBackendState,
	appId: string,
): Promise<UiInspectPackageWidget[]> {
	try {
		const packageWidgets = await listAppPackageWidgets(
			{
				listPackages: backend.appState.listPackages?.bind(backend.appState),
				getPackage: (packageId) => backend.registryState.getPackage(packageId),
			},
			appId,
		);
		return packageWidgets.map((entry) => ({
			selector: encodePackageWidgetRef(entry.packageId, entry.widget.id),
			package_id: entry.packageId,
			widget_id: entry.widget.id,
			package_version: entry.packageVersion,
			bundle_hash: entry.bundleHash,
			name: entry.widget.name,
			description: nonEmptyMetadataText(entry.widget.description),
			contract: entry.widget.contract,
		}));
	} catch {
		return [];
	}
}

/** @internal Exported only so tuple normalization and lookup stay regression-tested. */
export function resolveUiInspectWidgetEntries(
	list: readonly UiInspectWidgetListEntry[],
	selector?: string,
): {
	entries: readonly UiInspectWidgetEntry[];
	match: UiInspectWidgetEntry | undefined;
} {
	const entries = list.map(([, widgetId, metadata]) => ({
		widgetId,
		selector:
			nonEmptyMetadataText(metadata?.name) ??
			nonEmptyMetadataText(metadata?.title) ??
			widgetId,
		description: nonEmptyMetadataText(metadata?.description),
	}));
	const normalizedSelector = selector?.trim();
	return {
		entries,
		match: normalizedSelector
			? entries.find(
					(entry) =>
						entry.widgetId === normalizedSelector ||
						entry.selector === normalizedSelector,
				)
			: undefined,
	};
}

export const FRONTEND_RUNTIME_TOOL_NAMES = [
	"database_tool",
	"storage_tool",
	"ui_inspect",
	"execute_event",
	"execute_node",
	"query_execution_logs",
	"interact_app_page",
	"call_app_chat",
	"graph_overlay_tool",
	"graph_query_tool",
	"graph_element_tool",
	"ontology_action_tool",
] as const;

export type FrontendRuntimeToolName =
	(typeof FRONTEND_RUNTIME_TOOL_NAMES)[number];

interface FrontendRuntimeToolExecutorOptions {
	/** App context used when a board-scoped agent omits app_id. */
	defaultAppId?: string;
	/** Board context used when ui_inspect omits board_id. */
	defaultBoardId?: string;
	/** Overlay/ontology context used when a Data Studio tool omits overlay_id. */
	defaultOverlayId?: string;
}

function getArgString(
	args: Record<string, unknown>,
	snake: string,
	camel = snake,
): string | undefined {
	const value = args[snake] ?? args[camel];
	return typeof value === "string" && value.trim() ? value : undefined;
}

function getArgBool(
	args: Record<string, unknown>,
	snake: string,
	camel = snake,
	defaultValue = false,
): boolean {
	const value = args[snake] ?? args[camel];
	return typeof value === "boolean" ? value : defaultValue;
}

function getArgNumber(
	args: Record<string, unknown>,
	snake: string,
	camel = snake,
	defaultValue = 0,
): number {
	const value = args[snake] ?? args[camel];
	return typeof value === "number" && Number.isFinite(value)
		? value
		: defaultValue;
}

function clampToolLimit(value: number, defaultValue: number, maxValue: number) {
	if (!Number.isFinite(value) || value <= 0) return defaultValue;
	return Math.min(Math.floor(value), maxValue);
}

function parseDatabaseSchemaFields(value: unknown): IDatabaseSchemaField[] {
	if (!Array.isArray(value) || value.length === 0) {
		throw new Error("create_table requires a non-empty fields array.");
	}

	return value.map((entry, index) => {
		if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
			throw new Error(`create_table fields[${index}] must be an object.`);
		}
		const record = entry as Record<string, unknown>;
		const name = getArgString(record, "name");
		const type = getArgString(record, "type");
		if (!name || !type) {
			throw new Error(
				`create_table fields[${index}] requires non-empty name and type.`,
			);
		}

		const nullable = record.nullable;
		if (nullable !== undefined && typeof nullable !== "boolean") {
			throw new Error(
				`create_table fields[${index}].nullable must be a boolean.`,
			);
		}
		const vectorSize = record.vector_size ?? record.vectorSize;
		if (
			vectorSize !== undefined &&
			(typeof vectorSize !== "number" ||
				!Number.isInteger(vectorSize) ||
				vectorSize <= 0)
		) {
			throw new Error(
				`create_table fields[${index}].vector_size must be a positive integer.`,
			);
		}

		return {
			name,
			type,
			...(nullable === undefined ? {} : { nullable }),
			...(vectorSize === undefined
				? {}
				: { vector_size: vectorSize as number }),
		};
	});
}

/**
 * Create a table when the connected backend supports explicit schemas.
 *
 * A desktop frontend can temporarily be newer than its remote API during a rolling deploy. Older
 * APIs expose PUT/GET/DELETE on the table route but answer POST with 405. That is a capability
 * mismatch, not a reason to abort the board build: retain the explicit schema request and tell the
 * agent to continue building while setup remains pending.
 */
export async function createTableRuntime(
	dbState: Pick<IDatabaseState, "createTable">,
	options: {
		appId: string;
		tableName: string;
		fields: IDatabaseSchemaField[];
		ifNotExists: boolean;
		userScoped: boolean;
	},
) {
	const pendingSchema = {
		appId: options.appId,
		tableName: options.tableName,
		fields: options.fields,
		ifNotExists: options.ifNotExists,
		userScoped: options.userScoped,
	};
	if (isExplicitSchemaCreateUnavailable(options.appId)) {
		const pendingSchemaCount = retainPendingDatabaseSchema(pendingSchema);
		return explicitSchemaCreateUnavailableResult(
			options,
			pendingSchemaCount,
			true,
		);
	}

	try {
		const result = await dbState.createTable(
			options.appId,
			options.tableName,
			options.fields,
			options.ifNotExists,
			options.userScoped,
		);
		return {
			status: "ok" as const,
			user_scoped: options.userScoped,
			...result,
		};
	} catch (error) {
		if (error instanceof ApiResponseError && error.status === 405) {
			markExplicitSchemaCreateUnavailable(options.appId);
			const pendingSchemaCount = retainPendingDatabaseSchema(pendingSchema);
			return explicitSchemaCreateUnavailableResult(
				options,
				pendingSchemaCount,
				false,
			);
		}
		throw error;
	}
}

/**
 * Permanently drop a table. Irreversible, so the caller must repeat the table name: a truncated or
 * mis-templated argument must fail instead of destroying a table nobody named. The cascade report
 * is surfaced to the agent so it can tell the user which ontologies were pruned and which saved
 * queries now reference a missing table.
 */
export async function dropTableRuntime(
	dbState: Pick<IDatabaseState, "dropTable">,
	options: {
		appId: string;
		tableName: string;
		confirmTableName: string;
		userScoped: boolean;
	},
) {
	const tableName = normalizeDatabaseTableIdentifier(options.tableName);
	const confirmedTableName = normalizeDatabaseTableIdentifier(
		options.confirmTableName,
	);
	if (tableName !== confirmedTableName) {
		throw new Error(
			`delete_table confirmation mismatch: confirm_table_name '${options.confirmTableName}' does not match table_name '${options.tableName}'. Nothing was deleted.`,
		);
	}

	const result = await dbState.dropTable(
		options.appId,
		tableName,
		options.userScoped,
	);
	const ontologies = result.ontologies ?? [];
	const savedQueries = result.saved_queries ?? [];
	const warnings = result.warnings ?? [];
	const message = [
		result.dropped
			? `Table '${tableName}' and its schema were permanently deleted.`
			: `Table '${tableName}' did not exist; nothing was deleted.`,
		ontologies.length > 0
			? `Pruned references in ontology overlay(s): ${ontologies.join(", ")}.`
			: undefined,
		savedQueries.length > 0
			? `Saved queries still referencing this table (not deleted, they fail until edited): ${savedQueries.join(", ")}.`
			: undefined,
		warnings.length > 0 ? `Warnings: ${warnings.join(" | ")}` : undefined,
		"Report this cascade to the user.",
	]
		.filter((part): part is string => Boolean(part))
		.join(" ");

	return {
		status: "ok" as const,
		app_id: options.appId,
		table_name: result.table_name || tableName,
		user_scoped: options.userScoped,
		dropped: result.dropped,
		irreversible: true,
		ontologies_pruned: ontologies,
		saved_queries_referencing: savedQueries,
		warnings,
		message,
	};
}

function explicitSchemaCreateUnavailableResult(
	options: {
		appId: string;
		tableName: string;
		fields: IDatabaseSchemaField[];
		userScoped: boolean;
	},
	pendingSchemaCount: number,
	networkRequestSkipped: boolean,
) {
	return {
		status: "partial" as const,
		code: "explicit_schema_create_not_deployed",
		app_id: options.appId,
		table_name: options.tableName,
		created: false,
		user_scoped: options.userScoped,
		requested_fields: options.fields,
		pending_schema_count: pendingSchemaCount,
		network_request_skipped: networkRequestSkipped,
		message:
			"Explicit table-schema creation is unavailable on the connected API (the first POST returned 405). This schema is retained as pending in the frontend session. Submit each other required schema once; cached requests will not call the API or ask for approval. Continue the workflow/board build now and retry pending schemas only after the matching API is deployed and the frontend is reloaded. Do not replace the workflow with a database smoke test.",
		next_action: "continue_workflow_build",
	};
}

function resolveToolAppId(
	args: Record<string, unknown>,
	defaultAppId?: string,
): string {
	const appId = getArgString(args, "app_id", "appId") ?? defaultAppId;
	if (!appId) {
		throw new Error(
			"Missing app_id. Provide app_id or open FlowPilot from an app context.",
		);
	}
	return appId;
}

/**
 * A scoped session may target another app only when that app is visible in the current
 * profile — the same gate the global assistant applies before app-runtime tools.
 */
async function assertAppVisibleForRuntime(
	backend: IBackendState,
	appId: string,
	defaultAppId?: string,
): Promise<void> {
	if (!appId || appId === defaultAppId) return;
	let visible = false;
	try {
		const profile = await backend.userState.getSettingsProfile();
		visible = (profile?.hub_profile?.apps ?? []).some(
			(entry) => entry.app_id === appId,
		);
	} catch {
		throw new Error(
			`Could not verify that app '${appId}' is visible in the current profile; cross-app execution was not started.`,
		);
	}
	if (!visible) {
		throw new Error(`App '${appId}' is not visible in the current profile.`);
	}
}

/**
 * Mirror of `flow_like_ast::to_camel_case`: every run of non-alphanumeric
 * characters is a separator that is dropped and uppercases the next character.
 */
function toCamelCase(value: string): string {
	let out = "";
	let upcomingUpper = false;
	let first = true;
	for (const ch of value) {
		if (/[\p{L}\p{N}]/u.test(ch)) {
			if (first) {
				out += ch.toLowerCase();
				first = false;
			} else if (upcomingUpper) {
				out += ch.toUpperCase();
			} else {
				out += ch;
			}
			upcomingUpper = false;
		} else if (!first) {
			upcomingUpper = true;
		}
	}
	return out || "node";
}

function widgetInstantiatePin(
	kind: "path" | "prop" | "cust",
	key: string,
): string {
	return toCamelCase(`dyn_${kind}_${key}`);
}

function collectBoundPaths(components: SurfaceComponent[]): string[] {
	const paths = new Set<string>();
	const visit = (value: unknown) => {
		if (Array.isArray(value)) {
			for (const item of value) visit(item);
			return;
		}
		if (value && typeof value === "object") {
			const record = value as Record<string, unknown>;
			if (typeof record.path === "string") {
				if (record.path) paths.add(record.path);
				return;
			}
			for (const item of Object.values(record)) visit(item);
		}
	};
	for (const component of components ?? []) visit(component.component);
	return [...paths];
}

function summarizePage(page: IPage) {
	const customCss = page.canvasSettings?.customCss ?? "";
	return {
		page_id: page.id,
		name: page.name,
		route: page.route,
		on_load_event_id: page.onLoadEventId,
		on_interval_event_id: page.onIntervalEventId,
		// Size only, never the stylesheet itself — this tool feeds the orchestrator, and the UI
		// specialist receives the full customCss as its own context. Without this signal the
		// orchestrator cannot tell a styled page from an unstyled one.
		custom_css_chars: customCss.length,
		element_refs: (page.components ?? []).map(
			(component) => `${page.id}/${component.id}`,
		),
	};
}

function summarizeWidget(widget: IWidget) {
	return {
		selector: widget.name,
		widget_id: widget.id,
		description: widget.description,
		instantiate_pins: [
			...collectBoundPaths(widget.components ?? []).map((path) => ({
				pin: widgetInstantiatePin("path", path),
				bound_path: path,
			})),
			...(widget.exposedProps ?? []).map((prop) => ({
				pin: widgetInstantiatePin("prop", prop.id),
				label: prop.label,
				property_path: prop.propertyPath,
			})),
		],
		// Actions are persisted with `id` — that is also the string an events_widget_action node
		// matches on. Reading `name` here returned an empty list for every widget, leaving the
		// board specialist with no way to discover action ids except the instruction text.
		actions: (widget.actions ?? [])
			.map((action) => (action as { id?: string }).id)
			.filter((id): id is string => typeof id === "string" && id.length > 0),
	};
}

function mapIndexType(value: unknown): IIndexType {
	const normalized = String(value ?? "Auto")
		.replace(/[\s-]/g, "_")
		.toLowerCase();
	switch (normalized) {
		case "fulltext":
		case "full_text":
			return IIndexType.FullText;
		case "btree":
		case "b_tree":
			return IIndexType.BTree;
		case "bitmap":
			return IIndexType.Bitmap;
		case "labellist":
		case "label_list":
			return IIndexType.LabelList;
		default:
			return IIndexType.Auto;
	}
}

function splitStoragePath(path: string): { prefix: string; fileName: string } {
	const normalized = path.replace(/^\/+/, "");
	const lastSlash = normalized.lastIndexOf("/");
	if (lastSlash < 0) return { prefix: "", fileName: normalized };
	return {
		prefix: normalized.slice(0, lastSlash),
		fileName: normalized.slice(lastSlash + 1),
	};
}

type RuntimeBoardState = Pick<
	IBoardState,
	"executeBoard" | "getBoard" | "listRuns" | "queryRun"
>;

function buildRunPayload(targetId: string, value: unknown): IRunPayload {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		return { id: targetId, payload: {} };
	}

	const record = value as Record<string, unknown>;
	// Accept both the concise tool shape (`payload: {field: value}`) and an
	// explicit IRunPayload (`payload: {payload: {...}, runtime_variables: ...}`).
	if ("payload" in record || "runtime_variables" in record) {
		return { ...record, id: targetId } as IRunPayload;
	}
	return { id: targetId, payload: record };
}

function isLogMetadata(value: unknown): value is ILogMetadata {
	if (!value || typeof value !== "object") return false;
	const record = value as Record<string, unknown>;
	return (
		typeof record.run_id === "string" &&
		typeof record.app_id === "string" &&
		typeof record.board_id === "string"
	);
}

function compactExecutionLogs(logs: ILog[], maxLogs = 200): unknown[] {
	return logs.slice(0, maxLogs).map((log) => ({
		node_id: log.node_id,
		log_level: log.log_level,
		message: log.message,
		operation_id: log.operation_id,
		start: log.start,
		end: log.end,
		stats: log.stats ? compactJson(log.stats, 2000) : undefined,
	}));
}

function compactLogMetadata(metadata: ILogMetadata | undefined) {
	if (!metadata) return undefined;
	return {
		app_id: metadata.app_id,
		board_id: metadata.board_id,
		run_id: metadata.run_id,
		start: metadata.start,
		end: metadata.end,
		log_level: metadata.log_level,
		version: metadata.version,
		nodes: metadata.nodes?.slice(0, 100) ?? null,
		logs: metadata.logs ?? null,
		node_id: metadata.node_id,
		event_version: metadata.event_version ?? null,
		event_id: metadata.event_id,
		// queryRun only needs the run location; do not echo an arbitrarily large
		// execution payload into the model context.
		payload: [],
		payload_bytes: metadata.payload?.length ?? 0,
		is_remote: metadata.is_remote ?? false,
		nodes_truncated: (metadata.nodes?.length ?? 0) > 100,
	};
}

function logLevelName(log: ILog): string {
	return String(log.log_level).toLowerCase();
}

async function resolveRunMetadata(
	boardState: Pick<IBoardState, "listRuns">,
	appId: string,
	boardId: string,
	runId: string,
	provided: unknown,
): Promise<ILogMetadata> {
	if (isLogMetadata(provided)) {
		if (
			provided.run_id !== runId ||
			provided.app_id !== appId ||
			provided.board_id !== boardId
		) {
			throw new Error(
				"run_metadata does not match app_id, board_id, and run_id.",
			);
		}
		return provided;
	}

	const pageSize = 100;
	const maxRunsToScan = 1000;
	for (let offset = 0; offset < maxRunsToScan; offset += pageSize) {
		const runs = await boardState.listRuns(
			appId,
			boardId,
			undefined,
			undefined,
			undefined,
			undefined,
			undefined,
			offset,
			pageSize,
			false,
		);
		const match = runs.find((run) => run.run_id === runId);
		if (match) return match;
		if (runs.length < pageSize) break;
	}

	throw new Error(
		`Run '${runId}' was not found on board '${boardId}' (searched the latest ${maxRunsToScan} runs).`,
	);
}

export interface ExecuteNodeRuntimeArgs {
	appId: string;
	boardId: string;
	nodeId: string;
	payload?: unknown;
	streamState?: boolean;
	skipConsentCheck?: boolean;
}

/** Execute one board node as the run entry and retain a bounded live event tail. */
export async function executeNodeRuntime(
	boardState: Pick<IBoardState, "getBoard">,
	executeBoard: RuntimeBoardState["executeBoard"],
	args: ExecuteNodeRuntimeArgs,
) {
	const board = await boardState.getBoard(
		args.appId,
		args.boardId,
		undefined,
		true,
	);
	const node = board.nodes[args.nodeId];
	if (!node) {
		throw new Error(
			`Node '${args.nodeId}' was not found on board '${args.boardId}'.`,
		);
	}

	const liveEvents: unknown[] = [];
	let runId: string | undefined;
	const metadata = await executeBoard(
		args.appId,
		args.boardId,
		buildRunPayload(args.nodeId, args.payload),
		args.streamState ?? true,
		(id) => {
			runId = id;
		},
		(events) => {
			liveEvents.push(...events);
		},
		args.skipConsentCheck ?? false,
	);

	return {
		status: "ok",
		app_id: args.appId,
		board_id: args.boardId,
		node_id: args.nodeId,
		node_name: node.friendly_name || node.name,
		run_id: metadata?.run_id ?? runId,
		metadata: compactLogMetadata(metadata),
		live_event_count: liveEvents.length,
		live_events: compactLogEvents(liveEvents),
	};
}

export interface QueryExecutionLogsRuntimeArgs {
	appId: string;
	boardId: string;
	runId: string;
	runMetadata?: unknown;
	filter?: string;
	offset?: number;
	limit?: number;
}

/** Resolve a run and read its persisted logs through the normal board backend. */
export async function queryExecutionLogsRuntime(
	boardState: Pick<IBoardState, "listRuns" | "queryRun">,
	args: QueryExecutionLogsRuntimeArgs,
) {
	const limit = clampToolLimit(args.limit ?? 100, 100, 100);
	const offset = Math.max(0, Math.floor(args.offset ?? 0));
	const metadata = await resolveRunMetadata(
		boardState,
		args.appId,
		args.boardId,
		args.runId,
		args.runMetadata,
	);
	const logs = await boardState.queryRun(
		metadata,
		args.filter ?? "",
		offset,
		limit,
	);
	const warningCount = logs.filter((log) => {
		const level = logLevelName(log);
		return level === "warn" || level === "2";
	}).length;
	const errorCount = logs.filter((log) => {
		const level = logLevelName(log);
		return level === "error" || level === "3";
	}).length;
	const fatalCount = logs.filter((log) => {
		const level = logLevelName(log);
		return level === "fatal" || level === "4";
	}).length;
	const hasMore = logs.length === limit;

	return {
		status: "ok",
		app_id: args.appId,
		board_id: args.boardId,
		run_id: args.runId,
		filter: args.filter ?? "",
		offset,
		limit,
		log_count: logs.length,
		has_more: hasMore,
		verification: {
			scope: "returned_page",
			complete: !hasMore,
			warning_count: warningCount,
			error_count: errorCount,
			fatal_count: fatalCount,
			has_errors: errorCount + fatalCount > 0 ? true : hasMore ? null : false,
		},
		metadata: compactLogMetadata(metadata),
		logs: compactExecutionLogs(logs, limit),
	};
}

/**
 * Executes app-scoped runtime tools for both the embedded FlowPilot and the
 * global assistant bridge. Approval remains the caller's responsibility.
 */
function getArgOverlayId(
	args: Record<string, unknown>,
	defaultOverlayId?: string,
): string {
	return (
		getArgString(args, "overlay_id", "overlayId") ?? defaultOverlayId ?? ""
	);
}

/**
 * Executes one Data Studio graph tool against `backend.graphState`. Read operations are silent;
 * mutating/execute operations were already approved before this runs. `appId`/`overlayId` are
 * resolved by the caller (arg value, else the current Data Studio page default).
 */
async function executeGraphTool(
	backend: IBackendState,
	toolName: FrontendRuntimeToolName,
	operation: string,
	args: Record<string, unknown>,
	appId: string,
	overlayId: string,
	userScoped: boolean,
): Promise<unknown> {
	const graphState: IGraphState = backend.graphState;
	const limit = getArgString(args, "limit")
		? getArgNumber(args, "limit")
		: (args.limit as number | undefined);

	switch (toolName) {
		case "graph_overlay_tool":
			switch (operation) {
				case "list_overlays":
					return {
						status: "ok",
						overlays: await graphState.listOverlays(appId, userScoped),
					};
				case "get_overlay":
					return {
						status: "ok",
						overlay: await graphState.getOverlay(appId, overlayId, userScoped),
					};
				case "get_schema":
					return {
						status: "ok",
						schema: await graphState.getSchema(appId, overlayId, userScoped),
					};
				case "validate_overlay":
					return {
						status: "ok",
						validation: await graphState.validateOverlay(
							appId,
							overlayId,
							userScoped,
							args.draft as GraphOverlay | undefined,
						),
					};
				case "create_overlay": {
					// `actions` and `exposed` are governed and intentionally omitted here.
					const payload: CreateOverlayPayload = {
						name: getArgString(args, "name") ?? "",
						description: getArgString(args, "description"),
						nodes: (args.nodes as CreateOverlayPayload["nodes"]) ?? [],
						edges: (args.edges as CreateOverlayPayload["edges"]) ?? [],
						object_views:
							args.object_views as CreateOverlayPayload["object_views"],
						bindings_enabled: getArgBool(
							args,
							"bindings_enabled",
							"bindingsEnabled",
							false,
						),
						default_limit: args.default_limit as number | undefined,
					};
					return {
						status: "ok",
						overlay: await graphState.createOverlay(appId, payload, userScoped),
					};
				}
				case "update_overlay": {
					const payload: UpdateOverlayPayload = {
						expected_updated_at: getArgString(
							args,
							"expected_updated_at",
							"expectedUpdatedAt",
						),
						name: getArgString(args, "name"),
						description: getArgString(args, "description"),
						nodes: args.nodes as UpdateOverlayPayload["nodes"],
						edges: args.edges as UpdateOverlayPayload["edges"],
						object_views:
							args.object_views as UpdateOverlayPayload["object_views"],
						bindings_enabled: args.bindings_enabled as boolean | undefined,
						default_limit: args.default_limit as number | undefined,
					};
					return {
						status: "ok",
						overlay: await graphState.updateOverlay(
							appId,
							overlayId,
							payload,
							userScoped,
						),
					};
				}
				case "delete_overlay":
					await graphState.deleteOverlay(appId, overlayId, userScoped);
					return { status: "ok" };
				default:
					return {
						status: "error",
						error: `Unsupported graph_overlay_tool operation '${operation}'.`,
					};
			}
		case "graph_query_tool":
			switch (operation) {
				case "cypher":
					return {
						status: "ok",
						rows: await graphState.cypher(
							appId,
							overlayId,
							{
								query: getArgString(args, "query") ?? "",
								params: args.params as Record<string, unknown> | undefined,
								limit,
							},
							userScoped,
						),
					};
				case "sql":
					return {
						status: "ok",
						rows: await graphState.sql(
							appId,
							overlayId,
							{ query: getArgString(args, "query") ?? "", limit },
							userScoped,
						),
					};
				case "neighbors":
					return {
						status: "ok",
						result: await graphState.neighbors(
							appId,
							overlayId,
							{
								label: getArgString(args, "label") ?? "",
								node_id: args.node_id ?? args.nodeId,
								depth: args.depth as number | undefined,
								direction: args.direction as NeighborsPayload["direction"],
								limit,
							},
							userScoped,
						),
					};
				case "subgraph":
					return {
						status: "ok",
						result: await graphState.subgraph(
							appId,
							overlayId,
							{
								seeds: (args.seeds as SubgraphPayload["seeds"]) ?? [],
								depth: args.depth as number | undefined,
								limit,
							},
							userScoped,
						),
					};
				case "paths":
					return {
						status: "ok",
						result: await graphState.paths(
							appId,
							overlayId,
							{
								from_label: getArgString(args, "from_label", "fromLabel") ?? "",
								from_id: args.from_id ?? args.fromId,
								to_label: getArgString(args, "to_label", "toLabel") ?? "",
								to_id: args.to_id ?? args.toId,
								max_depth: args.max_depth as number | undefined,
								limit,
							},
							userScoped,
						),
					};
				case "analytics":
					return {
						status: "ok",
						analytics: await graphState.analytics(
							appId,
							overlayId,
							limit,
							userScoped,
						),
					};
				case "search_nodes":
					return {
						status: "ok",
						nodes: await graphState.searchNodes(
							appId,
							overlayId,
							{ query: getArgString(args, "query") ?? "", limit },
							userScoped,
						),
					};
				case "sample":
					return {
						status: "ok",
						rows: await graphState.sample(
							appId,
							overlayId,
							getArgString(args, "label") ?? "",
							args.n as number | undefined,
							userScoped,
						),
					};
				default:
					return {
						status: "error",
						error: `Unsupported graph_query_tool operation '${operation}'.`,
					};
			}
		case "graph_element_tool": {
			const label = getArgString(args, "label") ?? "";
			const rows = (args.rows as Record<string, unknown>[]) ?? [];
			if (operation === "add_nodes") {
				return {
					status: "ok",
					...(await graphState.upsertNodes(
						appId,
						overlayId,
						{ label, rows },
						userScoped,
					)),
				};
			}
			if (operation === "add_edges") {
				return {
					status: "ok",
					...(await graphState.upsertEdges(
						appId,
						overlayId,
						{ label, rows },
						userScoped,
					)),
				};
			}
			return {
				status: "error",
				error: `Unsupported graph_element_tool operation '${operation}'.`,
			};
		}
		case "ontology_action_tool": {
			const actionId = getArgString(args, "action_id", "actionId") ?? "";
			switch (operation) {
				case "list_actions": {
					const overlay = await graphState.getOverlay(
						appId,
						overlayId,
						userScoped,
					);
					return {
						status: "ok",
						actions: (overlay.actions ?? []).map((action) => ({
							id: action.id,
							name: action.name,
							description: action.description,
							object_type: action.object_type,
							enabled: action.enabled,
							allow_bulk: action.allow_bulk,
						})),
					};
				}
				case "describe_action": {
					const overlay = await graphState.getOverlay(
						appId,
						overlayId,
						userScoped,
					);
					const action = (overlay.actions ?? []).find(
						(candidate) => candidate.id === actionId,
					);
					const isOffline = await backend.isOffline(appId);
					const prerun = isOffline
						? { oauth_requirements: [], signature: "" }
						: await graphState.prerunOntologyAction(appId, overlayId, actionId);
					return { status: "ok", action, prerun };
				}
				case "prerun_action": {
					const isOffline = await backend.isOffline(appId);
					return {
						status: "ok",
						prerun: isOffline
							? { oauth_requirements: [], signature: "" }
							: await graphState.prerunOntologyAction(
									appId,
									overlayId,
									actionId,
								),
					};
				}
				case "invoke_action": {
					let payload: InvokeOntologyActionPayload = {
						object_refs: (args.object_refs as OntologyObjectRef[]) ?? [],
						parameters: args.parameters as Record<string, unknown> | undefined,
						idempotency_key: getArgString(
							args,
							"idempotency_key",
							"idempotencyKey",
						),
					};
					const isOffline = await backend.isOffline(appId);
					if (!isOffline && backend.eventState.checkOAuthRequirements) {
						const [overlay, prerun] = await Promise.all([
							graphState.getOverlay(appId, overlayId, userScoped),
							graphState.prerunOntologyAction(appId, overlayId, actionId),
						]);
						const action = (overlay.actions ?? []).find(
							(candidate) => candidate.id === actionId,
						);
						const oauth = await backend.eventState.checkOAuthRequirements(
							appId,
							prerun.oauth_requirements,
						);
						if (oauth.missingProviders.length > 0) {
							window.dispatchEvent(
								new CustomEvent("flow:oauth-required", {
									detail: {
										missingProviders: oauth.missingProviders,
										appId,
										boardId: action?.board_id ?? "",
										nodeId: action?.start_node_id ?? "",
										payload,
									},
								}),
							);
							throw new Error(
								"OAuth authorization is required. Complete authorization, then confirm the action again.",
							);
						}
						payload = { ...payload, oauth_tokens: oauth.tokens };
					}
					return {
						status: "ok",
						run: await graphState.invokeOntologyAction(
							appId,
							overlayId,
							actionId,
							payload,
						),
					};
				}
				default:
					return {
						status: "error",
						error: `Unsupported ontology_action_tool operation '${operation}'.`,
					};
			}
		}
		default:
			return { status: "error", error: `Unsupported tool '${toolName}'.` };
	}
}

export function useFrontendRuntimeToolExecutor(
	options: FrontendRuntimeToolExecutorOptions = {},
) {
	const { defaultAppId, defaultBoardId, defaultOverlayId } = options;
	const backend = useBackend();
	const executionService = useExecutionServiceOptional();

	return useCallback(
		async (
			toolName: FrontendRuntimeToolName,
			args: Record<string, unknown>,
		): Promise<unknown> => {
			const toolAppId = resolveToolAppId(args, defaultAppId);

			switch (toolName) {
				case "database_tool": {
					const operation = getArgString(args, "operation") ?? "list_tables";
					const tableName = getArgString(args, "table_name", "tableName");
					const userScoped = getArgBool(
						args,
						"user_scoped",
						"userScoped",
						false,
					);
					const offset = Math.max(0, getArgNumber(args, "offset", "offset", 0));
					const limit = clampToolLimit(
						getArgNumber(
							args,
							"limit",
							"limit",
							operation === "describe_table" ? 10 : 50,
						),
						operation === "describe_table" ? 10 : 50,
						200,
					);
					const includeSample = getArgBool(
						args,
						"include_sample",
						"includeSample",
						true,
					);

					switch (operation) {
						case "list_tables": {
							const [projectTables, userTables] = await Promise.all([
								backend.dbState.listTables(toolAppId),
								backend.dbState.listTablesUser(toolAppId),
							]);
							const pendingSchemas = getPendingDatabaseSchemas().filter(
								(schema) => schema.appId === toolAppId,
							);
							return {
								status: "ok",
								app_id: toolAppId,
								project_tables: projectTables,
								user_tables: userTables,
								...(isExplicitSchemaCreateUnavailable(toolAppId)
									? {
											explicit_schema_create_supported: false,
											pending_schema_requests: pendingSchemas.map((schema) => ({
												table_name: schema.tableName,
												user_scoped: schema.userScoped,
												if_not_exists: schema.ifNotExists,
												fields: schema.fields,
											})),
										}
									: {}),
							};
						}
						case "create_table": {
							if (!tableName)
								throw new Error("create_table requires table_name.");
							const fields = parseDatabaseSchemaFields(args.fields);
							const physicalTableName =
								normalizeDatabaseTableIdentifier(tableName);
							const result = await createTableRuntime(backend.dbState, {
								appId: toolAppId,
								tableName: physicalTableName,
								fields,
								ifNotExists: getArgBool(
									args,
									"if_not_exists",
									"ifNotExists",
									true,
								),
								userScoped,
							});
							return physicalTableName === tableName
								? result
								: {
										...result,
										table_name: physicalTableName,
										requested_table_name: tableName,
										name_normalized: true,
										name_normalization:
											"Human-facing table labels are stored as stable physical identifiers; use table_name in all workflow references.",
									};
						}
						case "delete_table": {
							if (!tableName)
								throw new Error("delete_table requires table_name.");
							const confirmTableName = getArgString(
								args,
								"confirm_table_name",
								"confirmTableName",
							);
							if (!confirmTableName) {
								throw new Error(
									"delete_table requires confirm_table_name repeating table_name exactly.",
								);
							}
							return await dropTableRuntime(backend.dbState, {
								appId: toolAppId,
								tableName,
								confirmTableName,
								userScoped,
							});
						}
						case "describe_table": {
							if (!tableName)
								throw new Error("describe_table requires table_name.");
							const [schema, indices, rowCount, sample] = await Promise.all([
								backend.dbState.getSchema(toolAppId, tableName, userScoped),
								backend.dbState.getIndices(toolAppId, tableName, userScoped),
								backend.dbState.countItems(toolAppId, tableName, userScoped),
								includeSample
									? backend.dbState.listItems(
											toolAppId,
											tableName,
											0,
											limit,
											userScoped,
										)
									: Promise.resolve(undefined),
							]);
							return {
								status: "ok",
								table_name: tableName,
								user_scoped: userScoped,
								schema,
								indices,
								row_count: rowCount,
								...(includeSample ? { sample } : {}),
							};
						}
						case "query": {
							if (!tableName) throw new Error("query requires table_name.");
							const query =
								args.query && typeof args.query === "object"
									? (args.query as Record<string, unknown>)
									: {};
							const rows = await backend.dbState.queryItems(
								toolAppId,
								tableName,
								query,
								offset,
								limit,
								userScoped,
							);
							return {
								status: "ok",
								table_name: tableName,
								user_scoped: userScoped,
								row_count: rows.length,
								rows,
							};
						}
						case "insert":
						case "add_items": {
							if (!tableName) throw new Error("insert requires table_name.");
							const items = Array.isArray(args.items) ? args.items : [];
							if (items.length === 0)
								throw new Error("insert requires non-empty items.");
							await backend.dbState.addItems(
								toolAppId,
								tableName,
								items,
								userScoped,
							);
							return {
								status: "ok",
								inserted: items.length,
								table_name: tableName,
							};
						}
						case "delete":
						case "remove_items": {
							if (!tableName) throw new Error("delete requires table_name.");
							const filter = getArgString(args, "filter");
							if (!filter) throw new Error("delete requires filter.");
							await backend.dbState.removeItems(
								toolAppId,
								tableName,
								filter,
								userScoped,
							);
							return { status: "ok", table_name: tableName, filter };
						}
						case "update": {
							if (!tableName) throw new Error("update requires table_name.");
							const filter = getArgString(args, "filter");
							const updates =
								args.updates && typeof args.updates === "object"
									? (args.updates as Record<string, unknown>)
									: undefined;
							if (!filter || !updates) {
								throw new Error("update requires filter and updates.");
							}
							await backend.dbState.updateItem(
								toolAppId,
								tableName,
								filter,
								updates,
								userScoped,
							);
							return { status: "ok", table_name: tableName, filter };
						}
						case "build_index": {
							if (!tableName)
								throw new Error("build_index requires table_name.");
							const column = getArgString(args, "column");
							if (!column) throw new Error("build_index requires column.");
							await backend.dbState.buildIndex(
								toolAppId,
								tableName,
								column,
								mapIndexType(args.index_type ?? args.indexType),
								getArgBool(args, "optimize", "optimize", false),
								userScoped,
							);
							return { status: "ok", table_name: tableName, column };
						}
						case "drop_index": {
							if (!tableName)
								throw new Error("drop_index requires table_name.");
							const indexName = getArgString(args, "index_name", "indexName");
							if (!indexName)
								throw new Error("drop_index requires index_name.");
							await backend.dbState.dropIndex(
								toolAppId,
								tableName,
								indexName,
								userScoped,
							);
							return {
								status: "ok",
								table_name: tableName,
								index_name: indexName,
							};
						}
						case "optimize": {
							if (!tableName) throw new Error("optimize requires table_name.");
							await backend.dbState.optimize(
								toolAppId,
								tableName,
								getArgBool(args, "keep_versions", "keepVersions", false),
								userScoped,
							);
							return { status: "ok", table_name: tableName };
						}
						case "add_column": {
							if (!tableName)
								throw new Error("add_column requires table_name.");
							const column =
								args.column_definition &&
								typeof args.column_definition === "object"
									? (args.column_definition as {
											name: string;
											sql_expression: string;
										})
									: undefined;
							if (!column?.name || !column?.sql_expression) {
								throw new Error(
									"add_column requires column_definition.name and sql_expression.",
								);
							}
							await backend.dbState.addColumn(
								toolAppId,
								tableName,
								column,
								userScoped,
							);
							return {
								status: "ok",
								table_name: tableName,
								column: column.name,
							};
						}
						case "drop_columns": {
							if (!tableName)
								throw new Error("drop_columns requires table_name.");
							const columns = Array.isArray(args.columns)
								? args.columns.filter(
										(value): value is string => typeof value === "string",
									)
								: [];
							if (columns.length === 0)
								throw new Error("drop_columns requires columns.");
							await backend.dbState.dropColumns(
								toolAppId,
								tableName,
								columns,
								userScoped,
							);
							return { status: "ok", table_name: tableName, columns };
						}
						case "alter_column": {
							if (!tableName)
								throw new Error("alter_column requires table_name.");
							const column = getArgString(args, "column");
							if (!column) throw new Error("alter_column requires column.");
							await backend.dbState.alterColumn(
								toolAppId,
								tableName,
								column,
								getArgBool(args, "nullable", "nullable", true),
								userScoped,
							);
							return { status: "ok", table_name: tableName, column };
						}
						default:
							throw new Error(
								`Unsupported database_tool operation '${operation}'.`,
							);
					}
				}

				case "storage_tool": {
					const operation = getArgString(args, "operation") ?? "list_files";
					const userScoped = getArgBool(
						args,
						"user_scoped",
						"userScoped",
						false,
					);
					const storage = backend.storageState;
					const list = userScoped
						? storage.listStorageItemsUser.bind(storage)
						: storage.listStorageItems.bind(storage);
					const download = userScoped
						? storage.downloadStorageItemsUser.bind(storage)
						: storage.downloadStorageItems.bind(storage);
					const upload = userScoped
						? storage.uploadStorageItemsUser.bind(storage)
						: storage.uploadStorageItems.bind(storage);
					const remove = userScoped
						? storage.deleteStorageItemsUser.bind(storage)
						: storage.deleteStorageItems.bind(storage);

					switch (operation) {
						case "list_files": {
							const prefix = getArgString(args, "prefix") ?? "";
							const items = await list(toolAppId, prefix);
							return {
								status: "ok",
								prefix,
								user_scoped: userScoped,
								items,
							};
						}
						case "read_file": {
							const path = getArgString(args, "path");
							if (!path) throw new Error("read_file requires path.");
							const maxChars = clampToolLimit(
								getArgNumber(args, "max_chars", "maxChars", 20_000),
								20_000,
								120_000,
							);
							const [file] = await download(toolAppId, [path]);
							if (!file || file.error) {
								throw new Error(
									file?.error ?? `Unable to resolve storage path '${path}'.`,
								);
							}
							if (!file.url) {
								return {
									status: "ok",
									path,
									message: "Storage provider returned no readable URL.",
								};
							}
							const response = await fetch(file.url);
							const content = await response.text();
							return {
								status: "ok",
								path,
								url: file.url,
								truncated: content.length > maxChars,
								content: content.slice(0, maxChars),
								chars: content.length,
							};
						}
						case "create_file": {
							const path = getArgString(args, "path");
							if (!path) throw new Error("create_file requires path.");
							const content = String(args.content ?? "");
							const mimeType =
								getArgString(args, "mime_type", "mimeType") ?? "text/plain";
							const { prefix, fileName } = splitStoragePath(path);
							if (!fileName)
								throw new Error("create_file path must include a file name.");
							const file = new File([content], fileName, { type: mimeType });
							await upload(toolAppId, prefix, [file]);
							return {
								status: "ok",
								path,
								bytes: new Blob([content]).size,
								user_scoped: userScoped,
							};
						}
						case "delete_files": {
							const fallbackPath = getArgString(args, "path");
							const paths = Array.isArray(args.paths)
								? args.paths.filter(
										(value): value is string => typeof value === "string",
									)
								: fallbackPath
									? [fallbackPath]
									: [];
							if (paths.length === 0)
								throw new Error("delete_files requires paths.");
							await remove(toolAppId, paths);
							return {
								status: "ok",
								deleted: paths,
								user_scoped: userScoped,
							};
						}
						default:
							throw new Error(
								`Unsupported storage_tool operation '${operation}'.`,
							);
					}
				}

				case "execute_event": {
					const eventId = getArgString(args, "event_id", "eventId");
					if (!eventId) throw new Error("execute_event requires event_id.");
					const streamState = getArgBool(
						args,
						"stream_state",
						"streamState",
						true,
					);
					// OAuth/RPA consent is a user gate; the model must never skip it, so the
					// former skip_consent_check argument is deliberately not read here.
					const skipConsentCheck = false;
					const payload = buildRunPayload(eventId, args.payload);
					const logs: unknown[] = [];
					let runId: string | undefined;
					const execute =
						executionService?.executeEvent ??
						backend.eventState.executeEvent.bind(backend.eventState);
					const metadata = await execute(
						toolAppId,
						eventId,
						payload as Parameters<typeof backend.eventState.executeEvent>[2],
						streamState,
						(id) => {
							runId = id;
						},
						(events) => {
							logs.push(...events);
						},
						skipConsentCheck,
					);
					return {
						status: "ok",
						app_id: toolAppId,
						event_id: eventId,
						run_id: metadata?.run_id ?? runId,
						metadata: compactLogMetadata(metadata),
						log_count: logs.length,
						logs: compactLogEvents(logs),
					};
				}

				case "execute_node": {
					const boardId =
						getArgString(args, "board_id", "boardId") ?? defaultBoardId;
					if (!boardId) throw new Error("execute_node requires board_id.");
					const nodeId = getArgString(args, "node_id", "nodeId");
					if (!nodeId) throw new Error("execute_node requires node_id.");
					const execute =
						executionService?.executeBoard ??
						backend.boardState.executeBoard.bind(backend.boardState);
					return executeNodeRuntime(backend.boardState, execute, {
						appId: toolAppId,
						boardId,
						nodeId,
						payload: args.payload,
						streamState: getArgBool(args, "stream_state", "streamState", true),
						skipConsentCheck: false,
					});
				}

				case "query_execution_logs": {
					const boardId =
						getArgString(args, "board_id", "boardId") ?? defaultBoardId;
					if (!boardId)
						throw new Error("query_execution_logs requires board_id.");
					const runId = getArgString(args, "run_id", "runId");
					if (!runId) throw new Error("query_execution_logs requires run_id.");
					return queryExecutionLogsRuntime(backend.boardState, {
						appId: toolAppId,
						boardId,
						runId,
						runMetadata: args.run_metadata ?? args.runMetadata,
						filter: getArgString(args, "filter") ?? getArgString(args, "query"),
						offset: getArgNumber(args, "offset", "offset", 0),
						limit: getArgNumber(args, "limit", "limit", 100),
					});
				}

				case "interact_app_page": {
					const actions = parseInteractActions(args.actions);
					if (actions.length === 0)
						throw new Error(
							"interact_app_page requires a non-empty actions array of {action: 'set_value'|'trigger', component_id, value?, event?}.",
						);
					await assertAppVisibleForRuntime(backend, toolAppId, defaultAppId);
					return interactWithAppPage(backend, {
						appId: toolAppId,
						eventId: getArgString(args, "event_id", "eventId"),
						pageId: getArgString(args, "page_id", "pageId"),
						actions,
						captureScreenshots: getArgBool(
							args,
							"capture_screenshots",
							"captureScreenshots",
							true,
						),
					});
				}

				case "call_app_chat": {
					const message =
						getArgString(args, "message") ?? getArgString(args, "prompt");
					if (!message) throw new Error("call_app_chat requires a message.");
					if (
						Array.isArray(args.forward_files) &&
						args.forward_files.length > 0
					) {
						throw new Error(
							"Attachment forwarding (forward_files) is only available in the global assistant chat; this session has no user-turn attachments. Retry without forward_files.",
						);
					}
					await assertAppVisibleForRuntime(backend, toolAppId, defaultAppId);
					return runAppChatMessage(backend, {
						appId: toolAppId,
						eventId: getArgString(args, "event_id", "eventId"),
						message,
					});
				}

				case "ui_inspect": {
					const operation = getArgString(args, "operation") ?? "list";
					const boardId =
						getArgString(args, "board_id", "boardId") ?? defaultBoardId;

					switch (operation) {
						case "page": {
							const pageId = getArgString(args, "page_id", "pageId");
							if (!pageId) {
								throw new Error(
									"ui_inspect operation 'page' requires page_id.",
								);
							}
							const page = await backend.pageState.getPage(
								toolAppId,
								pageId,
								boardId,
							);
							return { status: "ok", page: summarizePage(page) };
						}
						case "widget": {
							const selector = getArgString(
								args,
								"widget_selector",
								"widgetSelector",
							);
							if (!selector) {
								throw new Error(
									"ui_inspect operation 'widget' requires widget_selector.",
								);
							}
							const [list, packageWidgets] = await Promise.all([
								backend.widgetState.getWidgets(toolAppId),
								loadUiInspectPackageWidgets(backend, toolAppId),
							]);
							const normalizedSelector = selector.trim();
							// A `pkg:` selector is unambiguous, so resolve it directly. A bare name
							// stays project-first: a package widget must never shadow a project
							// widget that already answered to that name.
							const packageRef = packageWidgets.find(
								(entry) => entry.selector === normalizedSelector,
							);
							if (packageRef) {
								return { status: "ok", package_widget: packageRef };
							}
							const { match } = resolveUiInspectWidgetEntries(list, selector);
							if (!match) {
								const namedPackageWidget = packageWidgets.find(
									(entry) => entry.name === normalizedSelector,
								);
								if (namedPackageWidget) {
									return { status: "ok", package_widget: namedPackageWidget };
								}
								throw new Error(`Widget '${selector}' not found.`);
							}
							const widget = await backend.widgetState.getWidget(
								toolAppId,
								match.widgetId,
							);
							return { status: "ok", widget: summarizeWidget(widget) };
						}
						case "widgets": {
							const [list, packageWidgets] = await Promise.all([
								backend.widgetState.getWidgets(toolAppId),
								loadUiInspectPackageWidgets(backend, toolAppId),
							]);
							const { entries } = resolveUiInspectWidgetEntries(list);
							const widgets = await Promise.all(
								entries.map(async ({ widgetId, selector }) => {
									try {
										const widget = await backend.widgetState.getWidget(
											toolAppId,
											widgetId,
										);
										return summarizeWidget(widget);
									} catch {
										return {
											widget_id: widgetId,
											name: selector,
											error: "failed to load",
										};
									}
								}),
							);
							return { status: "ok", widgets, package_widgets: packageWidgets };
						}
						default: {
							const [pageList, widgetList, packageWidgets] = await Promise.all([
								backend.pageState.getPages(toolAppId, boardId),
								backend.widgetState.getWidgets(toolAppId),
								loadUiInspectPackageWidgets(backend, toolAppId),
							]);
							const pages = await Promise.all(
								pageList.map(async (item) => {
									try {
										const page = await backend.pageState.getPage(
											toolAppId,
											item.pageId,
											item.boardId ?? boardId,
										);
										return summarizePage(page);
									} catch {
										return {
											page_id: item.pageId,
											name: item.name,
											element_refs: [],
										};
									}
								}),
							);
							return {
								status: "ok",
								app_id: toolAppId,
								board_id: boardId,
								pages,
								widgets: resolveUiInspectWidgetEntries(widgetList).entries.map(
									({ widgetId, selector, description }) => ({
										selector,
										widget_id: widgetId,
										description,
									}),
								),
								package_widgets: packageWidgets,
							};
						}
					}
				}
				case "graph_overlay_tool":
				case "graph_query_tool":
				case "graph_element_tool":
				case "ontology_action_tool": {
					const operation = getArgString(args, "operation") ?? "";
					const overlayId = getArgOverlayId(args, defaultOverlayId);
					const userScoped = getArgBool(
						args,
						"user_scoped",
						"userScoped",
						false,
					);
					return executeGraphTool(
						backend,
						toolName,
						operation,
						args,
						toolAppId,
						overlayId,
						userScoped,
					);
				}
			}
		},
		[backend, defaultAppId, defaultBoardId, defaultOverlayId, executionService],
	);
}
