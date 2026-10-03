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
import {
	MAX_OWNED_TEMPLATE_METADATA,
	TemplateMetadataReadError,
	type TemplateReadCoverage,
	templateReadErrorMessage,
} from "../../state/backend-state/template-read";

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

function summarizeTemplateMetadata(metadata?: IMetadata) {
	if (!metadata) return undefined;
	let truncated = false;
	const trim = (value: string | null | undefined, limit: number) => {
		if (value && value.length > limit) truncated = true;
		return value?.slice(0, limit);
	};
	const tags = metadata.tags ?? [];
	if (tags.length > 12) truncated = true;
	const summary = {
		name: trim(metadata.name, 240),
		description: trim(metadata.description, 2_000),
		use_case: trim(metadata.use_case, 1_000),
		tags: tags.slice(0, 12).map((tag) => trim(tag, 80)),
	};
	return { ...summary, metadata_truncated: truncated };
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

	const publicOffset = args.public_offset ?? 0;
	const ownedOffset = args.owned_offset ?? 0;
	if (
		typeof publicOffset !== "number" ||
		!Number.isSafeInteger(publicOffset) ||
		publicOffset < 0 ||
		publicOffset > Number.MAX_SAFE_INTEGER - 100 ||
		typeof ownedOffset !== "number" ||
		!Number.isSafeInteger(ownedOffset) ||
		ownedOffset < 0 ||
		ownedOffset > Number.MAX_SAFE_INTEGER - 100
	) {
		return {
			status: "error",
			message:
				"Template offsets must be nonnegative integers no greater than Number.MAX_SAFE_INTEGER - 100.",
		};
	}
	const limit = Math.max(1, Math.floor(clampLimit(args.limit)));
	const publicLimit = Math.min(limit + 1, 100);
	const category =
		typeof args.category === "string"
			? (args.category as IAppCategory)
			: undefined;
	const tag = typeof args.tag === "string" ? args.tag : undefined;
	const forkableOnly = args.forkable_only === true;
	let ownedCoverage: TemplateReadCoverage = {
		complete: false,
		scope: "observed_owned_template_metadata",
		warning: "The backend did not certify exhaustive owned template coverage.",
	};
	const [publicRead, ownedRead] = await Promise.allSettled([
		backend.templateState.searchTemplates(
			{
				query,
				category,
				tag,
				forkable_only: forkableOnly,
				limit: publicLimit,
				offset: publicOffset,
			},
			{ strict: true, readOnly: true },
		),
		backend.templateState.getTemplates(undefined, undefined, {
			strict: true,
			readOnly: true,
			onCoverage: (coverage) => {
				ownedCoverage = coverage;
			},
		}),
	]);
	const publicError =
		publicRead.status === "rejected"
			? templateReadErrorMessage(publicRead.reason)
			: undefined;
	const ownedError =
		ownedRead.status === "rejected"
			? templateReadErrorMessage(ownedRead.reason)
			: undefined;
	const publicHits = publicRead.status === "fulfilled" ? publicRead.value : [];
	const inventory =
		ownedRead.status === "fulfilled"
			? ownedRead.value
			: ownedRead.reason instanceof TemplateMetadataReadError
				? ownedRead.reason.partialTemplates
				: [];
	const warnings = [ownedCoverage.warning].filter(
		(value): value is string => !!value,
	);
	if (inventory.length > MAX_OWNED_TEMPLATE_METADATA) {
		ownedCoverage.complete = false;
		warnings.push(
			`Searched at most ${MAX_OWNED_TEMPLATE_METADATA} owned metadata entries.`,
		);
	}
	const ownedMatches = new Map<string, (typeof inventory)[number]>();
	const normalizedQuery = query.toLowerCase();
	for (const entry of inventory.slice(0, MAX_OWNED_TEMPLATE_METADATA)) {
		const metadata = entry[2];
		if (
			!metadata ||
			![metadata.name, metadata.description].some((value) =>
				value?.toLowerCase().includes(normalizedQuery),
			) ||
			(tag && !metadata.tags?.includes(tag))
		)
			continue;
		ownedMatches.set(JSON.stringify([entry[0], entry[1]]), entry);
	}
	let ownedEntries = [...ownedMatches.values()].sort((a, b) => {
		const left = JSON.stringify([a[0], a[1]]);
		const right = JSON.stringify([b[0], b[1]]);
		return left < right ? -1 : left > right ? 1 : 0;
	});
	// Read authoritative app metadata only when an app-level filter needs it.
	if (category || forkableOnly) {
		const appIds = [...new Set(ownedEntries.map(([appId]) => appId))];
		const inspectedIds = appIds.slice(0, MAX_SEARCH_RESULTS);
		const apps = await Promise.allSettled(
			inspectedIds.map((id) => backend.appState.getAppAuthoritative(id)),
		);
		const accepted = new Set<string>();
		let failed = 0;
		for (const [index, result] of apps.entries()) {
			if (result.status === "rejected") {
				failed++;
				if (failed <= 3)
					warnings.push(
						`App filter read failed: ${templateReadErrorMessage(result.reason)}`,
					);
				continue;
			}
			const app = result.value;
			if (
				(!category ||
					app.primary_category === category ||
					app.secondary_category === category) &&
				(!forkableOnly || app.allow_forking === true)
			)
				accepted.add(inspectedIds[index]);
		}
		if (failed || appIds.length > inspectedIds.length) {
			ownedCoverage.complete = false;
			warnings.push(
				`App-level filters checked at most ${MAX_SEARCH_RESULTS} apps; ${failed} app reads failed. Unverified matches were omitted.`,
			);
		}
		ownedEntries = ownedEntries.filter(([appId]) => accepted.has(appId));
	}

	type Source = "owned" | "public";
	type Match = {
		app_id: string;
		template_id: string;
		sources: Source[];
		[key: string]: unknown;
	};
	const candidates: Record<Source, Match[]> = {
		public: publicHits.slice(0, publicLimit).map((hit) => ({
			app_id: hit.app_id,
			template_id: hit.template_id,
			app_name: hit.app_name?.slice(0, 240),
			app_name_truncated: (hit.app_name?.length ?? 0) > 240,
			app_allow_forking: hit.app_allow_forking,
			app_price: hit.app_price,
			...summarizeTemplateMetadata(hit.metadata),
			sources: ["public"],
		})),
		owned: ownedEntries
			.slice(ownedOffset)
			.map(([appId, templateId, metadata]) => ({
				app_id: appId,
				template_id: templateId,
				...summarizeTemplateMetadata(metadata),
				sources: ["owned"],
			})),
	};
	const consumed = { owned: 0, public: 0 };
	const merged = new Map<string, Match>();
	// Advance each source only for consumed rows, including duplicates. Unused
	// prefetched rows remain available through that source's next offset.
	for (;;) {
		let advanced = false;
		for (const source of ["owned", "public"] as const) {
			const candidate = candidates[source][consumed[source]];
			if (!candidate) continue;
			const key = JSON.stringify([candidate.app_id, candidate.template_id]);
			const previous = merged.get(key);
			if (!previous && merged.size >= limit) continue;
			merged.set(key, {
				...previous,
				...candidate,
				sources: [...new Set([...(previous?.sources ?? []), source])],
			});
			consumed[source]++;
			advanced = true;
		}
		if (!advanced) break;
	}
	const observedPublicExhausted = consumed.public === publicHits.length;
	const observedOwnedExhausted = consumed.owned === candidates.owned.length;
	const ownedComplete =
		ownedRead.status === "fulfilled" &&
		ownedCoverage.complete &&
		observedOwnedExhausted;
	return {
		status:
			publicRead.status === "rejected" &&
			ownedRead.status === "rejected" &&
			merged.size === 0
				? "error"
				: "ok",
		templates: [...merged.values()],
		complete: false,
		note: "Metadata search only. Reuse both numeric next_offset values to continue. observed_exhausted describes the returned window or owned inventory, not exhaustive account or public search. Public offsets may overlap when the API consolidates metadata rows.",
		coverage: {
			public: {
				offset: publicOffset,
				next_offset: publicOffset + consumed.public,
				complete: false,
				observed_exhausted: observedPublicExhausted,
				scope: "public_api_window",
				warning:
					"The public API does not report a cursor or exhaustion metadata. Short pages can result from metadata consolidation or concurrent deletions.",
				error: publicError,
			},
			owned: {
				offset: ownedOffset,
				next_offset: ownedOffset + consumed.owned,
				complete: ownedComplete,
				observed_exhausted: observedOwnedExhausted,
				scope: ownedCoverage.scope,
				warning: warnings.join(" ") || undefined,
				error: ownedError,
			},
		},
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

const NO_PURCHASE_NOTE =
	"This app cannot sell packages. A package the preview blocks as `paid` cannot be bought here, so report it as one the user cannot get.";

/** `purchasingAllowed` is false on builds that may not sell anything. */
export async function scoutForkPreview(
	backend: IBackendState,
	args: Record<string, unknown>,
	purchasingAllowed: boolean,
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
			...(purchasingAllowed ? {} : { note: NO_PURCHASE_NOTE }),
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
