import type { FrontendRuntimeToolName } from "../../hooks/use-frontend-runtime-tool-executor";

export const FLOWPILOT_BOARD_CONTEXT_AUGMENTATION_SCHEMA =
	"flowpilot.board-context-augmentation/v1" as const;

const MAX_TABLES = 48;
const MAX_FIELDS_PER_TABLE = 96;
const MAX_INDICES_PER_TABLE = 48;
const MAX_UI_ITEMS = 96;
const MAX_STORAGE_ITEMS = 96;
const MAX_MANIFEST_BYTES = 160_000;

type RuntimeToolExecutor = (
	toolName: FrontendRuntimeToolName,
	args: Record<string, unknown>,
) => Promise<unknown>;

export interface FlowPilotContextTruncation {
	resource:
		| "tables"
		| "schema_fields"
		| "schema_field_details"
		| "indices"
		| "pages"
		| "widgets"
		| "project_items"
		| "user_items"
		| "errors";
	available: number;
	included: number;
	reason: "collection_limit" | "transport_limit" | "transport_summarization";
	table_name?: string;
}

interface ContextSectionMetadata {
	complete: boolean;
	truncated: boolean;
	truncations: FlowPilotContextTruncation[];
	errors: string[];
}

export interface FlowPilotBoardContextAugmentation {
	schema: typeof FLOWPILOT_BOARD_CONTEXT_AUGMENTATION_SCHEMA;
	app_id: string;
	board_id: string;
	generated_at_ms: number;
	data: {
		complete: boolean;
		truncated: boolean;
		truncations: FlowPilotContextTruncation[];
		tables: Array<{
			table_name: string;
			user_scoped: boolean;
			schema?: unknown;
			indices?: unknown;
			error?: string;
		}>;
		errors: string[];
	};
	ui: {
		complete: boolean;
		truncated: boolean;
		truncations: FlowPilotContextTruncation[];
		pages: unknown[];
		widgets: unknown[];
		errors: string[];
	};
	storage: {
		complete: boolean;
		truncated: boolean;
		truncations: FlowPilotContextTruncation[];
		project_items: unknown[];
		user_items: unknown[];
		errors: string[];
	};
	truncated: boolean;
}

function objectValue(value: unknown): Record<string, unknown> {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}

function stringArray(value: unknown): string[] {
	return Array.isArray(value)
		? value
				.filter((item): item is string => typeof item === "string")
				.map((item) => item.trim())
				.filter(Boolean)
		: [];
}

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function stableValue(value: unknown): unknown {
	if (Array.isArray(value)) return value.map(stableValue);
	if (!value || typeof value !== "object") return value;
	return Object.fromEntries(
		Object.entries(value as Record<string, unknown>)
			.sort(([left], [right]) => left.localeCompare(right))
			.map(([key, item]) => [key, stableValue(item)]),
	);
}

function boundedString(value: unknown, maxLength = 512): string | undefined {
	if (typeof value !== "string") return undefined;
	return value.length <= maxLength
		? value
		: `${value.slice(0, Math.max(0, maxLength - 1))}…`;
}

function summarizeInventoryItem(value: unknown): unknown {
	const object = objectValue(value);
	const summaryKeys = [
		"id",
		"name",
		"title",
		"type",
		"path",
		"location",
		"key",
		"size",
		"mime_type",
		"content_type",
	] as const;
	const summary: Record<string, string | number | boolean> = {};
	for (const key of summaryKeys) {
		const item = object[key];
		if (typeof item === "string") {
			summary[key] = boundedString(item, 256) ?? "";
		} else if (typeof item === "number" || typeof item === "boolean") {
			summary[key] = item;
		}
	}
	if (Object.keys(summary).length > 0) return stableValue(summary);
	let serialized: string;
	try {
		serialized = JSON.stringify(stableValue(value));
	} catch {
		serialized = String(value);
	}
	return { summary: boundedString(serialized, 512) ?? "unavailable" };
}

function manifestByteLength(value: unknown): number {
	return new TextEncoder().encode(JSON.stringify(value)).length;
}

function truncationMessage(truncation: FlowPilotContextTruncation): string {
	const table = truncation.table_name
		? ` for table ${truncation.table_name}`
		: "";
	return `Context truncated: ${truncation.resource}${table} included ${truncation.included} of ${truncation.available} (${truncation.reason})`;
}

function truncationKey(truncation: FlowPilotContextTruncation): string {
	return [
		truncation.resource,
		truncation.table_name ?? "",
		truncation.available,
		truncation.included,
		truncation.reason,
	].join("\u001f");
}

function withTruncations<T extends ContextSectionMetadata>(
	section: T,
	additions: FlowPilotContextTruncation[],
): T {
	if (additions.length === 0) return section;
	const byKey = new Map(
		section.truncations.map((item) => [truncationKey(item), item]),
	);
	for (const item of additions) byKey.set(truncationKey(item), item);
	const truncations = [...byKey.values()].sort((left, right) =>
		truncationKey(left).localeCompare(truncationKey(right)),
	);
	const errors = [
		...section.errors,
		...additions.map(truncationMessage),
	].filter((item, index, all) => all.indexOf(item) === index);
	return {
		...section,
		complete: false,
		truncated: true,
		truncations,
		errors,
	};
}

function collectionTruncation(
	resource: FlowPilotContextTruncation["resource"],
	available: number,
	included: number,
	tableName?: string,
): FlowPilotContextTruncation | undefined {
	return available > included
		? {
				resource,
				available,
				included,
				reason: "collection_limit",
				...(tableName ? { table_name: tableName } : {}),
			}
		: undefined;
}

function transportTruncation(
	resource: FlowPilotContextTruncation["resource"],
	available: number,
	included: number,
	tableName?: string,
	reason: FlowPilotContextTruncation["reason"] = "transport_limit",
): FlowPilotContextTruncation | undefined {
	return available > included || reason === "transport_summarization"
		? {
				resource,
				available,
				included,
				reason,
				...(tableName ? { table_name: tableName } : {}),
			}
		: undefined;
}

function present<T>(value: T | undefined): value is T {
	return value !== undefined;
}

function boundSchema(
	value: unknown,
	tableName: string,
): { value: unknown; truncation?: FlowPilotContextTruncation } {
	const object = objectValue(value);
	const rawFields = Array.isArray(object.fields) ? object.fields : undefined;
	const fields = rawFields
		? rawFields.slice(0, MAX_FIELDS_PER_TABLE).map(stableValue)
		: undefined;
	return {
		value: stableValue({
			...object,
			...(fields ? { fields } : {}),
		}),
		truncation: rawFields
			? collectionTruncation(
					"schema_fields",
					rawFields.length,
					fields?.length ?? 0,
					tableName,
				)
			: undefined,
	};
}

function boundIndices(
	value: unknown,
	tableName: string,
): { value: unknown; truncation?: FlowPilotContextTruncation } {
	if (!Array.isArray(value)) return { value: stableValue(value) };
	const indices = value.slice(0, MAX_INDICES_PER_TABLE).map(stableValue);
	return {
		value: indices,
		truncation: collectionTruncation(
			"indices",
			value.length,
			indices.length,
			tableName,
		),
	};
}

function compactToByteLimit(
	manifest: FlowPilotBoardContextAugmentation,
): FlowPilotBoardContextAugmentation {
	if (manifestByteLength(manifest) <= MAX_MANIFEST_BYTES) {
		return manifest;
	}

	const omittedIndices = manifest.data.tables
		.map((table) => {
			const available = Array.isArray(table.indices) ? table.indices.length : 0;
			return transportTruncation("indices", available, 0, table.table_name);
		})
		.filter(present);
	const firstStorageTruncations = [
		transportTruncation(
			"project_items",
			manifest.storage.project_items.length,
			Math.min(manifest.storage.project_items.length, 24),
		),
		transportTruncation(
			"user_items",
			manifest.storage.user_items.length,
			Math.min(manifest.storage.user_items.length, 24),
		),
	].filter(present);
	let compact: FlowPilotBoardContextAugmentation = {
		...manifest,
		truncated: true,
		data: withTruncations(
			{
				...manifest.data,
				tables: manifest.data.tables.map((table) => ({
					table_name: table.table_name,
					user_scoped: table.user_scoped,
					...(table.schema ? { schema: table.schema } : {}),
					...(table.error ? { error: table.error } : {}),
				})),
			},
			omittedIndices,
		),
		storage: withTruncations(
			{
				...manifest.storage,
				project_items: manifest.storage.project_items.slice(0, 24),
				user_items: manifest.storage.user_items.slice(0, 24),
			},
			firstStorageTruncations,
		),
	};
	if (manifestByteLength(compact) <= MAX_MANIFEST_BYTES) return compact;

	const secondDataTruncations: FlowPilotContextTruncation[] = [
		transportTruncation(
			"tables",
			compact.data.tables.length,
			Math.min(compact.data.tables.length, 32),
		),
		transportTruncation(
			"errors",
			compact.data.errors.length,
			Math.min(compact.data.errors.length, 16),
		),
	].filter(present);
	const secondTables = compact.data.tables.slice(0, 32).map((table) => {
		const schema = objectValue(table.schema);
		const fields = Array.isArray(schema.fields) ? schema.fields : undefined;
		if (fields && fields.length > 0) {
			secondDataTruncations.push(
				...[
					transportTruncation(
						"schema_fields",
						fields.length,
						Math.min(fields.length, 32),
						table.table_name,
					),
					transportTruncation(
						"schema_field_details",
						Math.min(fields.length, 32),
						Math.min(fields.length, 32),
						table.table_name,
						"transport_summarization",
					),
				].filter(present),
			);
		}
		return {
			table_name: boundedString(table.table_name, 256) ?? "<unnamed>",
			user_scoped: table.user_scoped,
			...(table.schema
				? {
						schema: {
							...schema,
							fields: fields
								? fields.slice(0, 32).map(summarizeInventoryItem)
								: undefined,
						},
					}
				: {}),
			...(table.error ? { error: boundedString(table.error, 512) } : {}),
		};
	});
	const secondUiTruncations = [
		transportTruncation(
			"pages",
			compact.ui.pages.length,
			Math.min(compact.ui.pages.length, 32),
		),
		transportTruncation(
			"pages",
			Math.min(compact.ui.pages.length, 32),
			Math.min(compact.ui.pages.length, 32),
			undefined,
			"transport_summarization",
		),
		transportTruncation(
			"widgets",
			compact.ui.widgets.length,
			Math.min(compact.ui.widgets.length, 32),
		),
		transportTruncation(
			"widgets",
			Math.min(compact.ui.widgets.length, 32),
			Math.min(compact.ui.widgets.length, 32),
			undefined,
			"transport_summarization",
		),
		transportTruncation(
			"errors",
			compact.ui.errors.length,
			Math.min(compact.ui.errors.length, 16),
		),
	].filter((item): item is FlowPilotContextTruncation =>
		Boolean(
			item && (item.available > 0 || item.reason !== "transport_summarization"),
		),
	);
	const secondStorageTruncations = [
		transportTruncation(
			"project_items",
			compact.storage.project_items.length,
			Math.min(compact.storage.project_items.length, 24),
		),
		transportTruncation(
			"project_items",
			Math.min(compact.storage.project_items.length, 24),
			Math.min(compact.storage.project_items.length, 24),
			undefined,
			"transport_summarization",
		),
		transportTruncation(
			"user_items",
			compact.storage.user_items.length,
			Math.min(compact.storage.user_items.length, 24),
		),
		transportTruncation(
			"user_items",
			Math.min(compact.storage.user_items.length, 24),
			Math.min(compact.storage.user_items.length, 24),
			undefined,
			"transport_summarization",
		),
		transportTruncation(
			"errors",
			compact.storage.errors.length,
			Math.min(compact.storage.errors.length, 16),
		),
	].filter((item): item is FlowPilotContextTruncation =>
		Boolean(
			item && (item.available > 0 || item.reason !== "transport_summarization"),
		),
	);
	compact = {
		...compact,
		data: withTruncations(
			{
				...compact.data,
				tables: secondTables,
				errors: compact.data.errors
					.slice(0, 16)
					.map((error) => boundedString(error, 512) ?? "Unknown error"),
			},
			secondDataTruncations,
		),
		ui: withTruncations(
			{
				...compact.ui,
				pages: compact.ui.pages.slice(0, 32).map(summarizeInventoryItem),
				widgets: compact.ui.widgets.slice(0, 32).map(summarizeInventoryItem),
				errors: compact.ui.errors
					.slice(0, 16)
					.map((error) => boundedString(error, 512) ?? "Unknown error"),
			},
			secondUiTruncations,
		),
		storage: withTruncations(
			{
				...compact.storage,
				project_items: compact.storage.project_items
					.slice(0, 24)
					.map(summarizeInventoryItem),
				user_items: compact.storage.user_items
					.slice(0, 24)
					.map(summarizeInventoryItem),
				errors: compact.storage.errors
					.slice(0, 16)
					.map((error) => boundedString(error, 512) ?? "Unknown error"),
			},
			secondStorageTruncations,
		),
	};
	if (manifestByteLength(compact) <= MAX_MANIFEST_BYTES) return compact;

	const thirdDataTruncations: FlowPilotContextTruncation[] = [
		transportTruncation(
			"tables",
			compact.data.tables.length,
			Math.min(compact.data.tables.length, 24),
		),
	].filter(present);
	for (const table of compact.data.tables.slice(0, 24)) {
		const fields = objectValue(table.schema).fields;
		if (Array.isArray(fields) && fields.length > 0) {
			thirdDataTruncations.push({
				resource: "schema_fields",
				available: fields.length,
				included: 0,
				reason: "transport_limit",
				table_name: table.table_name,
			});
		}
	}
	const thirdUiTruncations = [
		transportTruncation(
			"pages",
			compact.ui.pages.length,
			Math.min(compact.ui.pages.length, 16),
		),
		transportTruncation(
			"widgets",
			compact.ui.widgets.length,
			Math.min(compact.ui.widgets.length, 16),
		),
	].filter(present);
	const thirdStorageTruncations = [
		transportTruncation(
			"project_items",
			compact.storage.project_items.length,
			Math.min(compact.storage.project_items.length, 16),
		),
		transportTruncation(
			"user_items",
			compact.storage.user_items.length,
			Math.min(compact.storage.user_items.length, 16),
		),
	].filter(present);
	compact = {
		...compact,
		data: withTruncations(
			{
				...compact.data,
				tables: compact.data.tables.slice(0, 24).map((table) => ({
					table_name: boundedString(table.table_name, 128) ?? "<unnamed>",
					user_scoped: table.user_scoped,
					...(table.error ? { error: boundedString(table.error, 256) } : {}),
				})),
			},
			thirdDataTruncations,
		),
		ui: withTruncations(
			{
				...compact.ui,
				pages: compact.ui.pages.slice(0, 16),
				widgets: compact.ui.widgets.slice(0, 16),
			},
			thirdUiTruncations,
		),
		storage: withTruncations(
			{
				...compact.storage,
				project_items: compact.storage.project_items.slice(0, 16),
				user_items: compact.storage.user_items.slice(0, 16),
			},
			thirdStorageTruncations,
		),
	};
	return compact;
}

/**
 * Gather the frontend-owned half of FlowPilot's board context once per invocation.
 *
 * Every model backend receives this same bounded payload through CopilotToolContext. Failures are
 * explicit and partial: callers never fall back to an unbounded table/page inventory loop.
 *
 * `cacheIdentity` is retained for source compatibility but intentionally does not enable a
 * process-wide TTL cache. Some callers can only provide board identity or aggregate node counts,
 * neither of which proves semantic freshness. A new run therefore recollects live context instead
 * of returning a potentially stale payload marked complete.
 */
export async function buildFlowPilotBoardContextAugmentation(
	execute: RuntimeToolExecutor,
	appId: string,
	boardId: string,
	_cacheIdentity: string,
): Promise<FlowPilotBoardContextAugmentation> {
	const generatedAtMs = Date.now();
	const dataErrors: string[] = [];
	const uiErrors: string[] = [];
	const storageErrors: string[] = [];
	const dataTruncations: FlowPilotContextTruncation[] = [];
	const uiTruncations: FlowPilotContextTruncation[] = [];
	const storageTruncations: FlowPilotContextTruncation[] = [];

	const [listedTables, uiResult, projectStorage, userStorage] =
		await Promise.allSettled([
			execute("database_tool", { operation: "list_tables", app_id: appId }),
			execute("ui_inspect", {
				operation: "list",
				app_id: appId,
				board_id: boardId,
			}),
			execute("storage_tool", {
				operation: "list_files",
				app_id: appId,
				prefix: "",
				user_scoped: false,
			}),
			execute("storage_tool", {
				operation: "list_files",
				app_id: appId,
				prefix: "",
				user_scoped: true,
			}),
		]);

	const tableRefs: Array<{ table_name: string; user_scoped: boolean }> = [];
	if (listedTables.status === "fulfilled") {
		const listed = objectValue(listedTables.value);
		for (const tableName of stringArray(listed.project_tables)) {
			tableRefs.push({ table_name: tableName, user_scoped: false });
		}
		for (const tableName of stringArray(listed.user_tables)) {
			tableRefs.push({ table_name: tableName, user_scoped: true });
		}
	} else {
		dataErrors.push(errorMessage(listedTables.reason));
	}
	tableRefs.sort((left, right) =>
		`${Number(left.user_scoped)}:${left.table_name}`.localeCompare(
			`${Number(right.user_scoped)}:${right.table_name}`,
		),
	);
	const tablesTruncated = tableRefs.length > MAX_TABLES;
	const tableTruncation = collectionTruncation(
		"tables",
		tableRefs.length,
		Math.min(tableRefs.length, MAX_TABLES),
	);
	if (tableTruncation) dataTruncations.push(tableTruncation);
	const describedResults = await Promise.all(
		tableRefs.slice(0, MAX_TABLES).map(async (table) => {
			try {
				const described = objectValue(
					await execute("database_tool", {
						operation: "describe_table",
						app_id: appId,
						table_name: table.table_name,
						user_scoped: table.user_scoped,
						include_sample: false,
					}),
				);
				const schema = boundSchema(described.schema, table.table_name);
				const indices = boundIndices(described.indices, table.table_name);
				return {
					table: {
						...table,
						schema: schema.value,
						indices: indices.value,
					},
					truncations: [schema.truncation, indices.truncation].filter(present),
				};
			} catch (error) {
				return {
					table: { ...table, error: errorMessage(error) },
					truncations: [] as FlowPilotContextTruncation[],
				};
			}
		}),
	);
	const describedTables: FlowPilotBoardContextAugmentation["data"]["tables"] =
		describedResults.map((result) => result.table);
	for (const result of describedResults) {
		dataTruncations.push(...result.truncations);
	}
	for (const table of describedTables) {
		if (table.error) dataErrors.push(`${table.table_name}: ${table.error}`);
	}

	const ui = uiResult.status === "fulfilled" ? objectValue(uiResult.value) : {};
	if (uiResult.status === "rejected")
		uiErrors.push(errorMessage(uiResult.reason));
	const projectStorageValue =
		projectStorage.status === "fulfilled"
			? objectValue(projectStorage.value)
			: {};
	if (projectStorage.status === "rejected") {
		storageErrors.push(errorMessage(projectStorage.reason));
	}
	const userStorageValue =
		userStorage.status === "fulfilled" ? objectValue(userStorage.value) : {};
	if (userStorage.status === "rejected") {
		storageErrors.push(errorMessage(userStorage.reason));
	}

	const pages = Array.isArray(ui.pages) ? ui.pages : [];
	const widgets = Array.isArray(ui.widgets) ? ui.widgets : [];
	const pageTruncation = collectionTruncation(
		"pages",
		pages.length,
		Math.min(pages.length, MAX_UI_ITEMS),
	);
	const widgetTruncation = collectionTruncation(
		"widgets",
		widgets.length,
		Math.min(widgets.length, MAX_UI_ITEMS),
	);
	if (pageTruncation) uiTruncations.push(pageTruncation);
	if (widgetTruncation) uiTruncations.push(widgetTruncation);

	const projectItems = Array.isArray(projectStorageValue.items)
		? projectStorageValue.items
		: [];
	const userItems = Array.isArray(userStorageValue.items)
		? userStorageValue.items
		: [];
	const projectItemsTruncation = collectionTruncation(
		"project_items",
		projectItems.length,
		Math.min(projectItems.length, MAX_STORAGE_ITEMS),
	);
	const userItemsTruncation = collectionTruncation(
		"user_items",
		userItems.length,
		Math.min(userItems.length, MAX_STORAGE_ITEMS),
	);
	if (projectItemsTruncation) storageTruncations.push(projectItemsTruncation);
	if (userItemsTruncation) storageTruncations.push(userItemsTruncation);

	const data = withTruncations(
		{
			complete: dataErrors.length === 0 && !tablesTruncated,
			truncated: false,
			truncations: [],
			tables: describedTables,
			errors: dataErrors,
		},
		dataTruncations,
	);
	const uiSection = withTruncations(
		{
			complete: uiErrors.length === 0,
			truncated: false,
			truncations: [],
			pages: pages.slice(0, MAX_UI_ITEMS).map(stableValue),
			widgets: widgets.slice(0, MAX_UI_ITEMS).map(stableValue),
			errors: uiErrors,
		},
		uiTruncations,
	);
	const storage = withTruncations(
		{
			complete: storageErrors.length === 0,
			truncated: false,
			truncations: [],
			project_items: projectItems.slice(0, MAX_STORAGE_ITEMS).map(stableValue),
			user_items: userItems.slice(0, MAX_STORAGE_ITEMS).map(stableValue),
			errors: storageErrors,
		},
		storageTruncations,
	);
	const manifest = compactToByteLimit({
		schema: FLOWPILOT_BOARD_CONTEXT_AUGMENTATION_SCHEMA,
		app_id: appId,
		board_id: boardId,
		generated_at_ms: generatedAtMs,
		data,
		ui: uiSection,
		storage,
		truncated: data.truncated || uiSection.truncated || storage.truncated,
	});
	return manifest;
}

export function clearFlowPilotBoardContextManifestCacheForTests(): void {
	// Kept as a no-op for compatibility with existing test and host integrations. The collector no
	// longer keeps a cross-run TTL cache because the legacy cache identity cannot prove freshness.
}
