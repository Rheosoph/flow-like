/**
 * Handlers for the Scout specialist's read-only research tools.
 *
 * These live outside `global-tool-bridge` because they are pure data-shaping over
 * the backend state, with no React or streaming involvement — and because every
 * one of them has the same job: produce a COMPACT digest. The Scout exists so
 * that raw boards, events and schemas never enter the orchestrator's context, so
 * the caps and summarisation here are the point, not an optimisation.
 *
 * Nothing in this module mutates. `fork_app` / `acquire_app` are orchestrator
 * tools handled in the bridge itself, so their approval prompts surface at the
 * top level.
 */

import type {
	IApp,
	IAppCategory,
	IForkPreviewTarget,
	IMetadata,
} from "../../lib";
import type { IBackendState } from "../../state/backend-state";

/** Keeps a pathological profile from flooding a single tool result. */
const MAX_SEARCH_RESULTS = 25;
const MAX_BOARDS = 40;
const MAX_EVENTS = 60;
const MAX_TABLES = 60;
const MAX_NODE_TYPES_PER_BOARD = 12;
const MAX_COLUMNS_PER_TABLE = 40;

export type ScoutSection =
	| "boards"
	| "events"
	| "tables"
	| "overlays"
	| "widgets"
	| "variables";

const ALL_SECTIONS: ScoutSection[] = [
	"boards",
	"events",
	"tables",
	"overlays",
	"widgets",
	"variables",
];

export interface ScoutToolResult {
	status: "ok" | "error";
	[key: string]: unknown;
}

function clampLimit(limit: unknown): number {
	const parsed =
		typeof limit === "number"
			? limit
			: typeof limit === "string"
				? Number.parseInt(limit, 10)
				: Number.NaN;
	if (!Number.isFinite(parsed) || parsed <= 0) return MAX_SEARCH_RESULTS;
	return Math.min(parsed, 100);
}

/** Metadata trimmed to what a foundation decision actually turns on. */
function summarizeMetadata(metadata?: IMetadata) {
	if (!metadata) return undefined;
	return {
		name: metadata.name,
		description: metadata.description,
		tags: metadata.tags ?? [],
		use_case: metadata.use_case,
	};
}

function summarizeApp(app: IApp, metadata?: IMetadata) {
	return {
		app_id: app.id,
		visibility: app.visibility,
		price: app.price ?? 0,
		allow_forking: app.allow_forking ?? false,
		forked_from: app.forked_from,
		primary_category: app.primary_category,
		avg_rating: app.avg_rating,
		rating_count: app.rating_count,
		...summarizeMetadata(metadata),
	};
}

export async function scoutSearchApps(
	backend: IBackendState,
	args: Record<string, unknown>,
): Promise<ScoutToolResult> {
	const query = typeof args.query === "string" ? args.query : "";
	if (!query) {
		return { status: "error", message: "search_apps requires a query." };
	}
	const category =
		typeof args.category === "string"
			? (args.category as IAppCategory)
			: undefined;
	const tag = typeof args.tag === "string" ? args.tag : undefined;
	const author = typeof args.author === "string" ? args.author : undefined;

	const results = await backend.appState.searchApps(
		undefined,
		query,
		undefined,
		category,
		author,
		undefined,
		tag,
		undefined,
		clampLimit(args.limit),
	);

	return {
		status: "ok",
		// Public store metadata only. Reading a non-member app's internals is not
		// possible, so the Scout must recommend acquire/fork rather than a splice.
		note: "Public store results are metadata only. Use inspect_app for apps the user is a member of.",
		apps: results.map(([app, metadata]) => summarizeApp(app, metadata)),
	};
}

export async function scoutGetAppDetail(
	backend: IBackendState,
	args: Record<string, unknown>,
): Promise<ScoutToolResult> {
	const appId = typeof args.app_id === "string" ? args.app_id : "";
	if (!appId) {
		return { status: "error", message: "get_app_detail requires an app_id." };
	}

	try {
		const [app, metadata] = await Promise.all([
			backend.appState.getApp(appId),
			backend.appState.getAppMeta(appId).catch(() => undefined),
		]);
		return {
			status: "ok",
			app: {
				...summarizeApp(app, metadata),
				long_description: metadata?.long_description,
				bits: app.bits?.length ?? 0,
				board_count: app.boards?.length ?? 0,
				event_count: app.events?.length ?? 0,
				template_count: app.templates?.length ?? 0,
			},
		};
	} catch (error) {
		return {
			status: "error",
			message: `Could not read app '${appId}': ${error instanceof Error ? error.message : String(error)}`,
		};
	}
}

export async function scoutSearchTemplates(
	backend: IBackendState,
	args: Record<string, unknown>,
): Promise<ScoutToolResult> {
	const query = typeof args.query === "string" ? args.query : "";
	if (!query) {
		return { status: "error", message: "search_templates requires a query." };
	}

	const hits = await backend.templateState.searchTemplates({
		query,
		category:
			typeof args.category === "string"
				? (args.category as IAppCategory)
				: undefined,
		tag: typeof args.tag === "string" ? args.tag : undefined,
		forkable_only: args.forkable_only === true,
		limit: clampLimit(args.limit),
	});

	return {
		status: "ok",
		templates: hits.map((hit) => ({
			app_id: hit.app_id,
			template_id: hit.template_id,
			app_name: hit.app_name,
			app_allow_forking: hit.app_allow_forking,
			app_price: hit.app_price,
			...summarizeMetadata(hit.metadata),
		})),
	};
}

export async function scoutGetTemplatePreview(
	backend: IBackendState,
	args: Record<string, unknown>,
): Promise<ScoutToolResult> {
	const appId = typeof args.app_id === "string" ? args.app_id : "";
	const templateId =
		typeof args.template_id === "string" ? args.template_id : "";
	if (!appId || !templateId) {
		return {
			status: "error",
			message: "get_template_preview requires app_id and template_id.",
		};
	}

	try {
		const preview = await backend.templateState.getTemplatePreview(
			appId,
			templateId,
		);
		return { status: "ok", preview };
	} catch (error) {
		return {
			status: "error",
			message: `Could not preview template '${templateId}': ${error instanceof Error ? error.message : String(error)}`,
		};
	}
}

export async function scoutForkPreview(
	backend: IBackendState,
	args: Record<string, unknown>,
): Promise<ScoutToolResult> {
	const appId = typeof args.app_id === "string" ? args.app_id : "";
	if (!appId) {
		return { status: "error", message: "fork_preview requires an app_id." };
	}
	const target: IForkPreviewTarget =
		args.target === "offline" ? "offline" : "online";

	try {
		const preview = await backend.appState.getForkPreview(appId, target);
		return {
			status: "ok",
			// The endpoint reports the permission verdict in the body rather than as
			// a 403, so `user_can_fork: false` is a normal answer to relay — not an
			// error to retry.
			preview,
		};
	} catch (error) {
		return {
			status: "error",
			message: `Could not preview a fork of '${appId}': ${error instanceof Error ? error.message : String(error)}`,
		};
	}
}

/**
 * Structured digest of ONE app the user is a member of. Summarises rather than
 * dumps: per board, the entry events and distinct node types plus counts; per
 * event, its declaration; per table, its column names and types.
 *
 * A permission failure on an individual section degrades that section instead of
 * failing the call — a partially readable app is still useful evidence.
 */
export async function scoutInspectApp(
	backend: IBackendState,
	args: Record<string, unknown>,
	isVisibleInProfile: (appId: string) => Promise<boolean>,
): Promise<ScoutToolResult> {
	const appId = typeof args.app_id === "string" ? args.app_id : "";
	if (!appId) {
		return { status: "error", message: "inspect_app requires an app_id." };
	}

	if (!(await isVisibleInProfile(appId))) {
		// An expected outcome for a public store app, not a failure. Saying so
		// explicitly stops the Scout proposing a fragment splice it cannot reach.
		return {
			status: "ok",
			inaccessible: true,
			app_id: appId,
			reason:
				"The user is not a member of this app, so its boards, events and tables cannot be read. Recommend acquire_app or fork_app instead of reusing a fragment from it.",
		};
	}

	const requested = Array.isArray(args.sections)
		? (args.sections.filter(
				(section): section is ScoutSection =>
					typeof section === "string" &&
					ALL_SECTIONS.includes(section as ScoutSection),
			) as ScoutSection[])
		: ALL_SECTIONS;
	const sections = requested.length > 0 ? requested : ALL_SECTIONS;
	const boardFilter = typeof args.board_id === "string" ? args.board_id : "";

	const digest: Record<string, unknown> = { status: "ok", app_id: appId };
	const unreadable: string[] = [];

	if (sections.includes("boards")) {
		try {
			// Summaries with node types carry everything the digest lists; the graphs
			// themselves would be megabytes the Scout never reads.
			const boards = await backend.boardState.getBoardSummaries(appId, [
				"node_types",
			]);
			const selected = (
				boardFilter
					? boards.filter((board) => board.id === boardFilter)
					: boards
			).slice(0, MAX_BOARDS);
			digest.boards = selected.map((board) => {
				const nodeTypes = board.nodeTypes ?? [];
				return {
					board_id: board.id,
					name: board.name,
					description: board.description,
					node_count: board.nodeCount,
					layer_count: board.layerCount,
					entry_nodes: (board.entryNodes ?? []).map((node) => ({
						node_id: node.nodeId,
						node_type: node.nodeType,
					})),
					node_types: nodeTypes.slice(0, MAX_NODE_TYPES_PER_BOARD),
					node_types_truncated: nodeTypes.length > MAX_NODE_TYPES_PER_BOARD,
				};
			});
			if (boards.length > MAX_BOARDS) digest.boards_truncated = true;
		} catch {
			unreadable.push("boards");
		}
	}

	if (sections.includes("events")) {
		try {
			const events = await backend.eventState.getEvents(appId);
			digest.events = events.slice(0, MAX_EVENTS).map((event) => ({
				event_id: event.id,
				name: event.name,
				description: event.description,
				event_type: event.event_type,
				board_id: event.board_id,
				node_id: event.node_id,
				route: event.route,
				active: event.active,
				execution_mode: event.execution_mode,
			}));
			if (events.length > MAX_EVENTS) digest.events_truncated = true;
		} catch {
			unreadable.push("events");
		}
	}

	if (sections.includes("tables")) {
		try {
			const tables = await backend.dbState.listTables(appId);
			const named = tables.slice(0, MAX_TABLES);
			digest.tables = await Promise.all(
				named.map(async (table) => {
					try {
						const schema = await backend.dbState.getSchema(appId, table);
						const fields = Array.isArray(
							(schema as { fields?: unknown[] })?.fields,
						)
							? ((schema as { fields: Record<string, unknown>[] }).fields ?? [])
							: [];
						return {
							name: table,
							columns: fields.slice(0, MAX_COLUMNS_PER_TABLE).map((field) => ({
								name: field.name,
								type: field.data_type ?? field.type,
							})),
							columns_truncated: fields.length > MAX_COLUMNS_PER_TABLE,
						};
					} catch {
						return { name: table, columns: [], schema_unreadable: true };
					}
				}),
			);
			if (tables.length > MAX_TABLES) digest.tables_truncated = true;
		} catch {
			unreadable.push("tables");
		}
	}

	if (sections.includes("overlays")) {
		try {
			const overlays = await backend.graphState.listOverlays(appId);
			digest.overlays = overlays.map((overlay) => ({
				overlay_id: overlay.id,
				name: overlay.name,
				description: overlay.description,
				node_types: (overlay.nodes ?? []).map((node) => node.label),
				edge_types: (overlay.edges ?? []).map((edge) => edge.label),
			}));
		} catch {
			unreadable.push("overlays");
		}
	}

	if (sections.includes("widgets")) {
		try {
			const widgets = await backend.widgetState.getWidgets(appId);
			digest.widgets = widgets.map(([, widgetId, metadata]) => ({
				widget_id: widgetId,
				name: metadata?.name,
				description: metadata?.description,
			}));
		} catch {
			unreadable.push("widgets");
		}
	}

	if (sections.includes("variables")) {
		try {
			const boards = await backend.boardState.getBoardVariables(appId);
			// Secret VALUES never leave the backend; the names still tell the Scout
			// which credentials a fork would need reconfigured.
			digest.variables = boards.slice(0, MAX_BOARDS).flatMap((board) =>
				Object.values(board.variables ?? {}).map((variable) => ({
					board_id: board.board_id,
					name: variable.name,
					data_type: variable.data_type,
					value_type: variable.value_type,
					secret: variable.secret ?? false,
				})),
			);
		} catch {
			unreadable.push("variables");
		}
	}

	if (unreadable.length > 0) digest.unreadable_sections = unreadable;
	return digest as ScoutToolResult;
}
