"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Boxes,
	Braces,
	Cloud,
	Database,
	Loader2,
	PanelLeftClose,
	PanelLeftOpen,
	Play,
	Plus,
	Save,
	SquareTerminal,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type ImperativePanelHandle, Panel } from "react-resizable-panels";
import { toast } from "sonner";
import { useAppPermissions } from "../../../../hooks/use-app-permissions";
import { useInvoke } from "../../../../hooks/use-invoke";
import { RolePermissions } from "../../../../lib/permission/role-permission";
import { asArray } from "../../../../lib/response-shape";
import { useBackend } from "../../../../state/backend-state";
import type {
	GraphOverlay,
	RemoteOntologyImport,
} from "../../../../state/backend-state/graph-state";
import type {
	CreateSavedQueryPayload,
	ExecuteSqlResult,
	QueryColumn,
	QuerySurface,
	SavedQuery,
	UpdateSavedQueryPayload,
	VizConfig,
} from "../../../../state/backend-state/query-state";
import { Badge } from "../../../ui/badge";
import { Button } from "../../../ui/button";
import { ResizableHandle, ResizablePanelGroup } from "../../../ui/resizable";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
} from "../../../ui/sheet";
import {
	type SqlCatalogTable,
	SqlEditor,
	extractParams,
	extractReferencedTables,
} from "../../../ui/sql-editor";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../../ui/tooltip";
import { SectionLockedPanel } from "../../permission/permission-gate";
import { PermissionNotice } from "../../permission/permission-notice";
import {
	OntologyActionParameterForm,
	humanizeIdentifier,
} from "../data-studio-panels";
import { EditorFooterBar, type LastRun } from "./editor-footer-bar";
import { QueryResultView } from "./query-result-view";
import { QueryTablePicker } from "./query-table-picker";
import { SaveQueryDialog } from "./save-query-dialog";
import { SavedQuerySidebar } from "./saved-query-sidebar";

const NARROW_WORKBENCH_PX = 720;
const DEFAULT_LIMIT = 1000;
const READ_DATA = [RolePermissions.ReadFiles, RolePermissions.ReadDatabase];
const WRITE_DATA = [RolePermissions.WriteFiles, RolePermissions.WriteDatabase];

function jsonValuesEqual(left: unknown, right: unknown): boolean {
	return JSON.stringify(left ?? null) === JSON.stringify(right ?? null);
}

interface ArrowField {
	name: string;
	type?: unknown;
	data_type?: unknown;
}

function arrowFieldsToColumns(schema: unknown): QueryColumn[] {
	const fields = (schema as { fields?: ArrowField[] })?.fields;
	if (!Array.isArray(fields)) return [];
	return fields.map((field, position) => {
		const rawType = field.type ?? field.data_type;
		const type_name =
			typeof rawType === "string"
				? rawType
				: rawType
					? JSON.stringify(rawType)
					: "";
		return { name: field.name, type_name, position };
	});
}

function overlayTableColumns(
	overlay: GraphOverlay | undefined,
): SqlCatalogTable[] {
	if (!overlay) return [];
	const tables: SqlCatalogTable[] = [];
	const seen = new Set<string>();
	const pushColumn = (
		columns: QueryColumn[],
		name?: string,
		type = "",
	): void => {
		if (name && !columns.some((column) => column.name === name)) {
			columns.push({ name, type_name: type, position: columns.length });
		}
	};
	for (const node of overlay.nodes) {
		if (seen.has(node.table)) continue;
		seen.add(node.table);
		const columns: QueryColumn[] = [];
		pushColumn(columns, node.id_column);
		pushColumn(columns, node.display_column);
		for (const property of node.property_columns)
			pushColumn(columns, property.name, property.data_type);
		tables.push({ name: node.table, columns });
	}
	for (const edge of overlay.edges) {
		if (seen.has(edge.table)) continue;
		seen.add(edge.table);
		const columns: QueryColumn[] = [];
		pushColumn(columns, edge.src_column);
		pushColumn(columns, edge.dst_column);
		for (const property of edge.property_columns)
			pushColumn(columns, property.name, property.data_type);
		tables.push({ name: edge.table, columns });
	}
	return tables;
}

type WorkbenchSurfaceKind = QuerySurface | "remote";

export function QueryWorkbench({
	appId,
	ontologies,
	remoteImports = [],
	resolveSourceName,
	projectTables,
	userTables,
	userScoped,
	onScopeChange,
}: Readonly<{
	appId: string;
	ontologies: GraphOverlay[];
	remoteImports?: RemoteOntologyImport[];
	resolveSourceName?: (targetAppId: string) => string;
	projectTables: string[];
	userTables: string[];
	userScoped: boolean;
	onScopeChange: (userScoped: boolean) => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	// Queries read data; saving and deleting them require write access.
	const permissions = useAppPermissions(appId);
	const canReadData = permissions.can(...READ_DATA);
	const canWriteData = permissions.can(...WRITE_DATA);
	const writeDeniedMessage = t(
		"savingQueriesNeedsWriteAccessToThisProjectsData",
		"Saving, duplicating or deleting a query writes to this project's data, which your role cannot do.",
	);
	const savedQueriesQuery = useInvoke(
		backend.queryState.listSavedQueries,
		backend.queryState,
		[appId, userScoped],
		canReadData,
	);
	const savedQueries = useMemo(
		() => asArray(savedQueriesQuery.data),
		[savedQueriesQuery.data],
	);

	const [sql, setSql] = useState("SELECT 1");
	const [draftName, setDraftName] = useState<string | null>(null);
	const [tablePickerOpen, setTablePickerOpen] = useState(false);
	const [surface, setSurface] = useState<WorkbenchSurfaceKind>("native");
	const [overlayId, setOverlayId] = useState<string | undefined>(undefined);
	const [remoteImportId, setRemoteImportId] = useState<string | undefined>(
		undefined,
	);
	const [paramValues, setParamValues] = useState<Record<string, unknown>>({});
	const [paramsValid, setParamsValid] = useState(true);
	const [paramsOpen, setParamsOpen] = useState(false);
	const [loadedParamSchema, setLoadedParamSchema] = useState<
		Record<string, unknown> | undefined
	>(undefined);
	const [vizConfig, setVizConfig] = useState<VizConfig>({ view: "table" });
	const [columnCache, setColumnCache] = useState<Record<string, QueryColumn[]>>(
		{},
	);
	const columnCacheRef = useRef(columnCache);
	columnCacheRef.current = columnCache;
	const [limit, setLimit] = useState<number | null>(DEFAULT_LIMIT);

	const [editing, setEditing] = useState<SavedQuery | null>(null);
	const [result, setResult] = useState<ExecuteSqlResult | null>(null);
	const [running, setRunning] = useState(false);
	const [runError, setRunError] = useState<string | null>(null);
	const [lastRun, setLastRun] = useState<LastRun | null>(null);
	const [cursor, setCursor] = useState<{ line: number; column: number }>();
	const [liveMessage, setLiveMessage] = useState("");

	const [sidebarCollapsed, setSidebarCollapsed] = useState(true);
	const [narrow, setNarrow] = useState(false);
	const [workbenchWidth, setWorkbenchWidth] = useState(1024);
	const [libraryDrawerOpen, setLibraryDrawerOpen] = useState(false);
	const workbenchRef = useRef<HTMLDivElement>(null);
	const editorPanelRef = useRef<ImperativePanelHandle>(null);
	const [resultsExpanded, setResultsExpanded] = useState(false);
	const runIdRef = useRef(0);
	const runningRef = useRef(false);
	const [resultSignature, setResultSignature] = useState<string | null>(null);
	const querySignature = JSON.stringify({
		appId,
		sql,
		surface,
		overlayId: surface === "overlay" ? overlayId : undefined,
		remoteImportId: surface === "remote" ? remoteImportId : undefined,
		paramValues,
		limit,
		userScoped,
	});
	const modified =
		editing !== null &&
		(sql !== editing.sql ||
			surface !== editing.surface ||
			(surface === "overlay" && overlayId !== editing.overlay_id) ||
			limit !== (editing.default_limit ?? DEFAULT_LIMIT) ||
			!jsonValuesEqual(vizConfig, editing.viz_config ?? { view: "table" }));
	const sidebarPanelRef = useRef<ImperativePanelHandle>(null);
	const paramFormRef = useRef<HTMLDivElement>(null);

	const [saveOpen, setSaveOpen] = useState(false);
	const [saveBusy, setSaveBusy] = useState(false);
	const [saveError, setSaveError] = useState<string | null>(null);

	const paramNames = useMemo(() => extractParams(sql), [sql]);
	const referencedTables = useMemo(() => extractReferencedTables(sql), [sql]);
	const activeNativeTables = useMemo(
		() => (userScoped ? userTables : projectTables),
		[userScoped, userTables, projectTables],
	);
	const columnCacheKey = useCallback(
		(table: string) => `${appId}:${userScoped ? "user" : "project"}:${table}`,
		[appId, userScoped],
	);

	const paramSchema = useMemo(() => {
		const loadedProps =
			(loadedParamSchema?.properties as Record<string, unknown>) ?? {};
		const properties: Record<string, unknown> = {};
		for (const name of paramNames) {
			properties[name] = loadedProps[name] ?? {
				type: "string",
				title: humanizeIdentifier(name),
			};
		}
		return { properties, required: paramNames };
	}, [paramNames, loadedParamSchema]);

	const paramsComplete = paramNames.every((name) => {
		const value = paramValues[name];
		const property = paramSchema.properties[name] as {
			type?: string | string[];
		};
		const allowsNull = Array.isArray(property.type)
			? property.type.includes("null")
			: property.type === "null";
		return (
			value !== undefined && value !== "" && (value !== null || allowsNull)
		);
	});

	// Drop values for parameters no longer present in the SQL.
	useEffect(() => {
		setParamValues((previous) => {
			const allowed = new Set(paramNames);
			const filtered = Object.fromEntries(
				Object.entries(previous).filter(([key]) => allowed.has(key)),
			);
			return Object.keys(filtered).length === Object.keys(previous).length
				? previous
				: filtered;
		});
	}, [paramNames]);

	// Lazily fetch column metadata for referenced native tables. Reads the cache
	// via a ref so cache writes don't re-trigger the effect (which would restart
	// the fetch loop and re-request the still-missing tables — O(n²) calls).
	useEffect(() => {
		if (surface !== "native" || !canReadData) return;
		const missing = referencedTables.filter(
			(table) =>
				activeNativeTables.includes(table) &&
				!(columnCacheKey(table) in columnCacheRef.current),
		);
		if (missing.length === 0) return;
		let cancelled = false;
		void (async () => {
			for (const table of missing) {
				const cacheKey = columnCacheKey(table);
				try {
					const schema = await backend.dbState.getSchema(
						appId,
						table,
						userScoped,
					);
					if (!cancelled)
						setColumnCache((prev) => ({
							...prev,
							[cacheKey]: arrowFieldsToColumns(schema),
						}));
				} catch {
					if (!cancelled)
						setColumnCache((prev) => ({ ...prev, [cacheKey]: [] }));
				}
			}
		})();
		return () => {
			cancelled = true;
		};
	}, [
		canReadData,
		referencedTables,
		surface,
		activeNativeTables,
		columnCacheKey,
		appId,
		backend.dbState,
		userScoped,
	]);

	const catalog = useMemo(() => {
		if (surface === "remote") {
			const imported = remoteImports.find((item) => item.id === remoteImportId);
			return {
				tables: overlayTableColumns(imported?.contract),
				views: [],
				params: paramNames,
			};
		}
		if (surface === "overlay") {
			const overlay = ontologies.find((item) => item.id === overlayId);
			return {
				tables: overlayTableColumns(overlay),
				views: savedQueries
					.filter(
						(query) =>
							query.kind === "view" &&
							query.surface === "overlay" &&
							query.overlay_id === overlayId,
					)
					.map((query) => ({ name: query.name })),
				params: paramNames,
			};
		}
		return {
			tables: activeNativeTables.map((name) => ({
				name,
				scope: userScoped ? ("user" as const) : ("project" as const),
				columns: columnCache[columnCacheKey(name)],
			})),
			views: savedQueries
				.filter((query) => query.kind === "view" && query.surface === "native")
				.map((query) => ({ name: query.name })),
			params: paramNames,
		};
	}, [
		surface,
		overlayId,
		remoteImportId,
		remoteImports,
		ontologies,
		activeNativeTables,
		columnCacheKey,
		columnCache,
		savedQueries,
		paramNames,
		userScoped,
	]);

	const runQuery = useCallback(async () => {
		if (
			!sql.trim() ||
			!canReadData ||
			runningRef.current ||
			(surface === "overlay" && !overlayId) ||
			(surface === "remote" && !remoteImportId) ||
			(paramNames.length > 0 && (!paramsValid || !paramsComplete))
		)
			return;
		const runId = ++runIdRef.current;
		runningRef.current = true;
		setRunning(true);
		setRunError(null);
		setLiveMessage("Running query…");
		const startedAt = performance.now();
		try {
			let value: ExecuteSqlResult;
			if (surface === "remote") {
				if (!remoteImportId) return;
				value = await backend.graphState.queryRemoteImport(
					appId,
					remoteImportId,
					{ sql, params: paramValues, limit: limit ?? undefined },
				);
			} else {
				value = await backend.queryState.executeSql(
					appId,
					{
						sql,
						params: paramValues,
						surface,
						overlay_id: surface === "overlay" ? overlayId : undefined,
						limit: limit ?? undefined,
					},
					userScoped,
				);
			}
			if (runId !== runIdRef.current) return;
			setResult(value);
			setResultSignature(querySignature);
			setLastRun({
				durationMs: performance.now() - startedAt,
				rowCount: value.row_count,
				ok: true,
			});
			setLiveMessage(
				`${value.row_count} ${value.row_count === 1 ? "row" : "rows"} returned`,
			);
		} catch (error) {
			if (runId !== runIdRef.current) return;
			setRunError(
				error instanceof Error
					? error.message
					: t("theQueryCouldNotRun", "The query could not run."),
			);
			setResult(null);
			setLastRun({
				durationMs: performance.now() - startedAt,
				rowCount: 0,
				ok: false,
			});
			setLiveMessage("Query failed");
		} finally {
			if (runId === runIdRef.current) {
				runningRef.current = false;
				setRunning(false);
			}
		}
	}, [
		appId,
		backend.queryState,
		backend.graphState,
		canReadData,
		limit,
		overlayId,
		remoteImportId,
		paramValues,
		paramNames.length,
		paramsValid,
		paramsComplete,
		querySignature,
		sql,
		surface,
		t,
		userScoped,
	]);

	const loadSavedQuery = useCallback((query: SavedQuery) => {
		runIdRef.current++;
		runningRef.current = false;
		setRunning(false);
		setLibraryDrawerOpen(false);
		editorPanelRef.current?.expand();
		setEditing(query);
		setSql(query.sql);
		setSurface(query.surface);
		setOverlayId(query.overlay_id);
		setLoadedParamSchema(query.param_schema);
		setParamValues({});
		setParamsValid(true);
		setParamsOpen(false);
		setVizConfig(query.viz_config ?? { view: "table" });
		setLimit(query.default_limit ?? DEFAULT_LIMIT);
		setResult(null);
		setRunError(null);
		setLastRun(null);
		setLiveMessage("");
	}, []);

	const startNewQuery = useCallback(() => {
		setDraftName(null);
		runIdRef.current++;
		runningRef.current = false;
		setRunning(false);
		setLibraryDrawerOpen(false);
		editorPanelRef.current?.expand();
		setEditing(null);
		setSql("SELECT 1");
		setLoadedParamSchema(undefined);
		setParamValues({});
		setParamsValid(true);
		setParamsOpen(false);
		setVizConfig({ view: "table" });
		setLimit(DEFAULT_LIMIT);
		setResult(null);
		setRunError(null);
		setLastRun(null);
		setLiveMessage("");
	}, []);
	const queryScope = `${appId}:${userScoped}`;
	const savedQueryScopeRef = useRef(queryScope);
	useEffect(() => {
		if (savedQueryScopeRef.current === queryScope) return;
		savedQueryScopeRef.current = queryScope;
		startNewQuery();
	}, [startNewQuery, queryScope]);
	useEffect(() => {
		if (surface !== "overlay") return;
		if (overlayId && ontologies.some((overlay) => overlay.id === overlayId)) {
			return;
		}
		setOverlayId(ontologies[0]?.id);
	}, [ontologies, overlayId, surface]);

	useEffect(() => {
		if (surface !== "remote") return;
		if (
			remoteImportId &&
			remoteImports.some((item) => item.id === remoteImportId)
		)
			return;
		setRemoteImportId(remoteImports[0]?.id);
	}, [remoteImports, remoteImportId, surface]);

	const deleteSavedQuery = useCallback(
		async (query: SavedQuery) => {
			await backend.queryState.deleteSavedQuery(appId, query.id, userScoped);
			if (editing?.id === query.id) startNewQuery();
			await savedQueriesQuery.refetch();
		},
		[
			appId,
			backend.queryState,
			editing,
			savedQueriesQuery,
			startNewQuery,
			userScoped,
		],
	);

	const handleSaveConfirm = useCallback(
		async (details: {
			name: string;
			description?: string;
			kind: "query" | "view";
		}) => {
			// Remote queries are ad-hoc read-through previews; they are not persisted.
			if (surface === "remote" || !canWriteData) return;
			setSaveBusy(true);
			setSaveError(null);
			const payload: CreateSavedQueryPayload = {
				name: details.name,
				description: details.description,
				kind: details.kind,
				surface,
				overlay_id: surface === "overlay" ? overlayId : undefined,
				sql,
				param_schema: paramNames.length > 0 ? paramSchema : undefined,
				viz_config: vizConfig,
				default_limit: limit ?? undefined,
			};
			try {
				let saved: SavedQuery;
				if (editing) {
					const update: UpdateSavedQueryPayload = {
						expected_updated_at: editing.updated_at,
					};
					if (payload.name !== editing.name) update.name = payload.name;
					if (payload.kind !== editing.kind) update.kind = payload.kind;
					if (payload.surface !== editing.surface)
						update.surface = payload.surface;
					if (payload.sql !== editing.sql) update.sql = payload.sql;

					const nextDescription = payload.description ?? null;
					if (nextDescription !== (editing.description ?? null)) {
						update.description = nextDescription;
					}
					const nextOverlayId = payload.overlay_id ?? null;
					if (nextOverlayId !== (editing.overlay_id ?? null)) {
						update.overlay_id = nextOverlayId;
					}
					const nextParamSchema = payload.param_schema ?? null;
					if (!jsonValuesEqual(nextParamSchema, editing.param_schema)) {
						update.param_schema = nextParamSchema;
					}
					const nextVizConfig = payload.viz_config ?? null;
					if (
						!jsonValuesEqual(
							nextVizConfig,
							editing.viz_config ?? { view: "table" },
						)
					) {
						update.viz_config = nextVizConfig;
					}
					const nextDefaultLimit = payload.default_limit ?? null;
					if (nextDefaultLimit !== (editing.default_limit ?? DEFAULT_LIMIT)) {
						update.default_limit = nextDefaultLimit;
					}

					saved = await backend.queryState.updateSavedQuery(
						appId,
						editing.id,
						update,
						userScoped,
					);
				} else {
					saved = await backend.queryState.createSavedQuery(
						appId,
						payload,
						userScoped,
					);
				}
				setEditing(saved);
				setSaveOpen(false);
				await savedQueriesQuery.refetch();
			} catch (error) {
				setSaveError(
					error instanceof Error
						? error.message
						: t("couldNotSaveTheQuery", "Could not save the query."),
				);
			} finally {
				setSaveBusy(false);
			}
		},
		[
			appId,
			backend.queryState,
			canWriteData,
			editing,
			limit,
			overlayId,
			paramNames.length,
			paramSchema,
			savedQueriesQuery,
			sql,
			surface,
			t,
			userScoped,
			vizConfig,
		],
	);

	const disabledReason = !canReadData
		? t(
				"yourRoleCannotRunQueriesOnThisProject",
				"Your role cannot run queries on this project",
			)
		: running
			? "Running…"
			: !sql.trim()
				? t("writeAQueryFirst", "Write a query first")
				: surface === "overlay" && !overlayId
					? t("selectAnOntology", "Select an ontology")
					: surface === "remote" && !remoteImportId
						? t("selectARemoteOntology", "Select a remote ontology")
						: paramNames.length > 0 && (!paramsValid || !paramsComplete)
							? t("fixParameterValues", "Fix parameter values")
							: null;
	const runDisabled = disabledReason !== null;

	const saveDisabledReason = !canWriteData
		? writeDeniedMessage
		: surface === "remote"
			? t(
					"remoteQueriesAreReadonlyPreviewsAndCannotBeSaved",
					"Remote queries are read-only previews and cannot be saved",
				)
			: null;

	const openSave = useCallback(() => {
		if (!sql.trim() || surface === "remote" || !canWriteData) return;
		setSaveError(null);
		setSaveOpen(true);
	}, [sql, surface, canWriteData]);

	const denyWrite = useCallback(() => {
		toast.error(writeDeniedMessage);
	}, [writeDeniedMessage]);

	useEffect(() => {
		const root = workbenchRef.current;
		if (!root) return;
		const observer = new ResizeObserver(([entry]) => {
			const isNarrow = entry.contentRect.width < NARROW_WORKBENCH_PX;
			setWorkbenchWidth(entry.contentRect.width || 1024);
			setNarrow(isNarrow);
			if (isNarrow) sidebarPanelRef.current?.collapse();
		});
		observer.observe(root);
		return () => observer.disconnect();
	}, []);

	useEffect(
		() => () => {
			runIdRef.current++;
		},
		[],
	);

	const toggleSidebar = useCallback(() => {
		if (narrow) {
			setLibraryDrawerOpen(true);
			return;
		}
		const panel = sidebarPanelRef.current;
		if (!panel) return;
		if (panel.isCollapsed()) panel.expand();
		else panel.collapse();
	}, [narrow]);

	const toggleResults = useCallback(() => {
		const panel = editorPanelRef.current;
		if (!panel) return;
		if (panel.isCollapsed()) panel.expand();
		else panel.collapse();
	}, []);

	const focusParamForm = useCallback(() => {
		if (narrow) {
			setParamsOpen(true);
			return;
		}
		const el = paramFormRef.current?.querySelector<HTMLElement>(
			"input, select, textarea, button",
		);
		el?.focus();
		el?.scrollIntoView({ block: "nearest" });
	}, [narrow]);

	const handleKeyDown = useCallback(
		(event: React.KeyboardEvent) => {
			if (!(event.metaKey || event.ctrlKey)) return;
			if (event.key === "Enter") {
				event.preventDefault();
				if (!runDisabled) void runQuery();
			} else if (event.key.toLowerCase() === "s") {
				event.preventDefault();
				openSave();
			}
		},
		[runDisabled, runQuery, openSave],
	);

	if (!canReadData && !permissions.isLoading) {
		return (
			<div className="flex h-full min-h-0 flex-col overflow-auto p-6">
				<SectionLockedPanel
					feature={t("queries", "Queries")}
					description={t(
						"runningSqlHereReadsThisProjectsData",
						"Running SQL here reads this project's tables and ontologies, which your role cannot do.",
					)}
					missing={READ_DATA}
					roleName={permissions.roleName}
				/>
			</div>
		);
	}

	const library = (
		<div className="flex h-full min-h-0 flex-col">
			{!canWriteData && (
				<PermissionNotice
					tone="readOnly"
					className="m-2 mb-0"
					title={t("savedQueriesAreReadonly", "Saved queries are read-only")}
					description={writeDeniedMessage}
					missing={WRITE_DATA}
				/>
			)}
			<div className="min-h-0 flex-1">
				<SavedQuerySidebar
					queries={savedQueries}
					activeId={editing?.id}
					loading={savedQueriesQuery.isLoading}
					onSelect={loadSavedQuery}
					onNew={startNewQuery}
					onDelete={canWriteData ? deleteSavedQuery : denyWrite}
					onDuplicate={
						canWriteData
							? async (query) => {
									await backend.queryState.createSavedQuery(
										appId,
										{
											name: `${query.name} copy`,
											description: query.description,
											kind: query.kind,
											surface: query.surface,
											overlay_id: query.overlay_id,
											sql: query.sql,
											param_schema: query.param_schema,
											viz_config: query.viz_config,
											default_limit: query.default_limit,
										},
										userScoped,
									);
									await savedQueriesQuery.refetch();
								}
							: denyWrite
					}
				/>
			</div>
		</div>
	);

	return (
		<div
			ref={workbenchRef}
			className="@container/workbench flex h-full min-h-0 flex-col overflow-hidden rounded-lg border bg-background"
			onKeyDown={handleKeyDown}
		>
			<output className="sr-only" aria-live="polite">
				{liveMessage}
			</output>

			<ResizablePanelGroup
				direction="horizontal"
				autoSaveId="qw-library-v2"
				className="min-h-0 flex-1"
			>
				<Panel
					id="qw-sidebar"
					order={1}
					ref={sidebarPanelRef}
					collapsible
					collapsedSize={0}
					defaultSize={0}
					minSize={Math.min(35, (220 / workbenchWidth) * 100)}
					maxSize={35}
					onCollapse={() => setSidebarCollapsed(true)}
					onExpand={() => setSidebarCollapsed(false)}
					className="min-w-0"
				>
					{!sidebarCollapsed && library}
				</Panel>

				<ResizableHandle
					disabled={narrow}
					withHandle={!sidebarCollapsed}
					className={sidebarCollapsed ? "w-0" : undefined}
					aria-label={t(
						"resizeSavedqueriesSidebar",
						"Resize saved-queries sidebar",
					)}
				/>

				<Panel id="qw-main" order={2} minSize={40} className="min-w-0">
					<section className="flex h-full min-h-0 min-w-0 flex-col">
						<header className="flex min-h-14 shrink-0 flex-wrap items-center gap-2 border-b px-3 py-2">
							<Button
								variant="ghost"
								size="sm"
								className="h-8 gap-1.5 px-2 text-muted-foreground"
								onClick={toggleSidebar}
								aria-expanded={narrow ? libraryDrawerOpen : !sidebarCollapsed}
								aria-label={t("toggleSavedQueries", "Toggle saved queries")}
							>
								{sidebarCollapsed ? (
									<PanelLeftOpen className="h-4 w-4" />
								) : (
									<PanelLeftClose className="h-4 w-4" />
								)}
								<span className="hidden @lg/workbench:inline">
									{t("savedQueries", "Saved queries")}
								</span>
								<span className="@lg/workbench:hidden">
									{t("saved", "Saved")}
								</span>
								{savedQueries.length > 0 && (
									<span className="text-[10px] tabular-nums">
										{savedQueries.length}
									</span>
								)}
							</Button>
							<div className="order-first w-full min-w-0 @lg/workbench:order-none @lg/workbench:w-auto @lg/workbench:flex-1 @lg/workbench:border-l @lg/workbench:pl-3">
								<div className="flex min-w-0 items-center gap-2">
									<SquareTerminal className="hidden h-4 w-4 shrink-0 text-muted-foreground @lg/workbench:block" />
									<h3
										className="truncate text-sm font-semibold"
										title={editing?.name}
									>
										{editing?.name ??
											draftName ??
											t("untitledQuery", "Untitled query")}
									</h3>
									{!editing && (
										<span className="rounded border bg-muted/40 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
											{t("draft", "Draft")}
										</span>
									)}
									{modified && (
										<span
											className="h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500"
											title={t("unsavedChanges", "Unsaved changes")}
										>
											<span className="sr-only">
												{t("unsavedChanges", "Unsaved changes")}
											</span>
										</span>
									)}
								</div>
							</div>
							<div className="ml-auto flex shrink-0 items-center gap-1.5">
								<Button
									variant="ghost"
									size="sm"
									className="h-8 gap-1 px-2"
									onClick={startNewQuery}
									aria-label={t("newQuery", "New query")}
								>
									<Plus className="h-4 w-4" />
									<span className="hidden @xl/workbench:inline">
										{t("new", "New")}
									</span>
								</Button>
								<Button
									variant="outline"
									size="sm"
									className="h-8 gap-1.5"
									disabled={!sql.trim() || saveDisabledReason !== null}
									title={saveDisabledReason ?? undefined}
									onClick={openSave}
								>
									<Save className="h-4 w-4" /> {t("save", "Save")}
								</Button>
								<Tooltip>
									<TooltipTrigger asChild>
										<span className="inline-flex">
											<Button
												size="sm"
												className="h-8 gap-1.5"
												disabled={runDisabled}
												onClick={() => void runQuery()}
											>
												{running ? (
													<Loader2 className="h-4 w-4 animate-spin" />
												) : (
													<Play className="h-4 w-4" />
												)}
												{running
													? t("runningQuery", "Running…")
													: t("run", "Run")}
												<kbd className="ml-1 hidden rounded border border-primary-foreground/30 px-1 text-[10px] @2xl/workbench:inline">
													⌘↵
												</kbd>
											</Button>
										</span>
									</TooltipTrigger>
									<TooltipContent>
										{disabledReason ?? "Run query"}
									</TooltipContent>
								</Tooltip>
							</div>
						</header>

						<ResizablePanelGroup
							direction="vertical"
							autoSaveId="qw-layout-v2"
							className="min-h-0 flex-1"
						>
							<Panel
								id="qw-editor"
								order={1}
								ref={editorPanelRef}
								collapsible
								collapsedSize={0}
								defaultSize={36}
								minSize={25}
								onCollapse={() => setResultsExpanded(true)}
								onExpand={() => setResultsExpanded(false)}
							>
								<div className="flex h-full min-h-0 flex-col">
									<div className="flex shrink-0 flex-wrap items-center gap-2 border-b bg-muted/20 px-3 py-2">
										<span className="text-[11px] font-medium text-muted-foreground">
											{t("source", "Source")}
										</span>
										<Select
											value={surface}
											onValueChange={(value) => {
												setSurface(value as WorkbenchSurfaceKind);
												if (value === "overlay" && !overlayId)
													setOverlayId(ontologies[0]?.id);
												if (value === "remote" && !remoteImportId)
													setRemoteImportId(remoteImports[0]?.id);
											}}
										>
											<SelectTrigger
												className="h-8 w-40 text-xs"
												aria-label={t("querySource", "Query source")}
											>
												<SelectValue />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="native">
													<span className="flex items-center gap-2">
														<Database className="h-3.5 w-3.5" />
														{t("nativeTables", "Native tables")}
													</span>
												</SelectItem>
												<SelectItem
													value="overlay"
													disabled={ontologies.length === 0}
												>
													<span className="flex items-center gap-2">
														<Boxes className="h-3.5 w-3.5" />
														{t("ontology", "Ontology")}
													</span>
												</SelectItem>
												<SelectItem
													value="remote"
													disabled={remoteImports.length === 0}
												>
													<span className="flex items-center gap-2">
														<Cloud className="h-3.5 w-3.5" />
														{t("remoteOntology", "Remote ontology")}
													</span>
												</SelectItem>
											</SelectContent>
										</Select>
										{surface !== "remote" && (
											<Select
												value={userScoped ? "user" : "project"}
												onValueChange={(value) =>
													onScopeChange(value === "user")
												}
											>
												<SelectTrigger
													className="h-8 w-24 text-xs"
													aria-label={t("databaseScope", "Database scope")}
												>
													<SelectValue />
												</SelectTrigger>
												<SelectContent>
													<SelectItem value="project">
														{t("project", "Project")}
													</SelectItem>
													<SelectItem value="user">
														{t("personal", "Personal")}
													</SelectItem>
												</SelectContent>
											</Select>
										)}

										{surface === "overlay" && (
											<Select value={overlayId} onValueChange={setOverlayId}>
												<SelectTrigger
													className="h-8 w-44"
													aria-label="Ontology"
												>
													<SelectValue
														placeholder={t("selectOntology", "Select ontology")}
													/>
												</SelectTrigger>
												<SelectContent>
													{ontologies.map((overlay) => (
														<SelectItem key={overlay.id} value={overlay.id}>
															{overlay.name}
														</SelectItem>
													))}
												</SelectContent>
											</Select>
										)}

										{surface === "remote" && (
											<>
												<Select
													value={remoteImportId}
													onValueChange={setRemoteImportId}
												>
													<SelectTrigger
														className="h-8 w-52"
														aria-label={t("remoteOntology", "Remote ontology")}
													>
														<SelectValue
															placeholder={t(
																"selectRemoteOntology",
																"Select remote ontology",
															)}
														/>
													</SelectTrigger>
													<SelectContent>
														{remoteImports.map((imported) => (
															<SelectItem key={imported.id} value={imported.id}>
																{imported.contract.name}
																{" · "}
																{resolveSourceName?.(imported.target_app_id) ??
																	imported.target_app_id}
															</SelectItem>
														))}
													</SelectContent>
												</Select>
												<Badge
													variant="outline"
													className="hidden gap-1 @xl/ds:flex"
												>
													<Cloud className="h-3 w-3" />{" "}
													{t("readonly", "Read-only")}
												</Badge>
											</>
										)}
										{paramNames.length > 0 && narrow && (
											<Button
												variant="outline"
												size="sm"
												className="h-8 gap-1.5 text-xs"
												onClick={() => setParamsOpen(true)}
											>
												<Braces className="h-3.5 w-3.5" />
												{t("parameters", "Parameters")} ({paramNames.length})
												{!paramsComplete && (
													<span className="h-1.5 w-1.5 rounded-full bg-amber-500" />
												)}
											</Button>
										)}
										<QueryTablePicker
											tables={catalog.tables}
											views={catalog.views}
											open={tablePickerOpen}
											onOpenChange={setTablePickerOpen}
											className="ml-auto"
											onSelectTable={(name) => {
												startNewQuery();
												setDraftName(name);
												setSql(
													`SELECT *\nFROM "${name.replaceAll('"', '""')}"`,
												);
											}}
										/>
									</div>
									<div className="flex min-h-0 flex-1">
										<div className="min-h-0 min-w-0 flex-1">
											<SqlEditor
												value={sql}
												onChange={setSql}
												catalog={catalog}
												onRun={() => {
													if (!runDisabled) void runQuery();
												}}
												onCursorChange={setCursor}
											/>
										</div>
										{paramNames.length > 0 && !narrow && (
											<div
												ref={paramFormRef}
												className="w-64 shrink-0 overflow-y-auto border-l p-3"
											>
												<OntologyActionParameterForm
													key={`${editing?.id ?? "draft"}:${paramNames.join(",")}`}
													actionId="workbench-query"
													schema={paramSchema}
													description={t(
														"queryParameterFormHint",
														"Fill in the parameters referenced in your SQL.",
													)}
													parameters={paramValues}
													disabled={running}
													onChange={setParamValues}
													onValidityChange={setParamsValid}
												/>
											</div>
										)}
									</div>
									<EditorFooterBar
										surface={surface}
										overlayName={
											surface === "remote"
												? remoteImports.find(
														(item) => item.id === remoteImportId,
													)?.contract.name
												: ontologies.find((item) => item.id === overlayId)?.name
										}
										params={paramNames}
										tables={referencedTables}
										cursor={cursor}
										limit={limit}
										onLimitChange={setLimit}
										onParamClick={focusParamForm}
									/>
								</div>
							</Panel>

							<ResizableHandle
								withHandle={!resultsExpanded}
								aria-label={t(
									"resizeEditorAndResults",
									"Resize editor and results",
								)}
							/>

							<Panel id="qw-results" order={2} minSize={30}>
								<QueryResultView
									appId={appId}
									result={result}
									lastRun={lastRun}
									stale={result !== null && resultSignature !== querySignature}
									resultsExpanded={resultsExpanded}
									onToggleResultsExpanded={toggleResults}
									runDisabled={runDisabled}
									loading={running}
									error={runError}
									vizConfig={vizConfig}
									onVizConfigChange={setVizConfig}
									onChooseTable={
										catalog.tables.length + catalog.views.length > 0
											? () => {
													editorPanelRef.current?.expand();
													setTablePickerOpen(true);
												}
											: undefined
									}
									onRun={() => void runQuery()}
								/>
							</Panel>
						</ResizablePanelGroup>
					</section>
				</Panel>
			</ResizablePanelGroup>

			<Sheet open={libraryDrawerOpen} onOpenChange={setLibraryDrawerOpen}>
				<SheetContent
					side="left"
					className="w-[min(360px,90vw)] gap-0 p-0 pt-10"
				>
					<SheetHeader className="sr-only">
						<SheetTitle>{t("savedQueries", "Saved queries")}</SheetTitle>
						<SheetDescription>
							{t(
								"chooseSavedQuery",
								"Choose a saved query or view to open in the editor.",
							)}
						</SheetDescription>
					</SheetHeader>
					{library}
				</SheetContent>
			</Sheet>

			<Sheet
				open={narrow && paramsOpen && paramNames.length > 0}
				onOpenChange={setParamsOpen}
			>
				<SheetContent className="w-full sm:max-w-md">
					<SheetHeader>
						<SheetTitle>{t("queryParameters", "Query parameters")}</SheetTitle>
						<SheetDescription>
							{t(
								"queryParameterHint",
								"Set the values used by this query before running it.",
							)}
						</SheetDescription>
					</SheetHeader>
					<div className="min-h-0 flex-1 overflow-auto px-4">
						<OntologyActionParameterForm
							actionId="workbench-query-compact"
							schema={paramSchema}
							description={t(
								"queryParameterFormHint",
								"Fill in the parameters referenced in your SQL.",
							)}
							parameters={paramValues}
							disabled={running}
							onChange={setParamValues}
							onValidityChange={setParamsValid}
						/>
					</div>
					<Button className="mx-4 mb-4" onClick={() => setParamsOpen(false)}>
						{t("done", "Done")}
					</Button>
				</SheetContent>
			</Sheet>

			<SaveQueryDialog
				open={saveOpen}
				onOpenChange={setSaveOpen}
				sql={sql}
				params={paramNames}
				busy={saveBusy}
				error={saveError}
				defaults={
					editing
						? {
								name: editing.name,
								description: editing.description,
								kind: editing.kind,
							}
						: undefined
				}
				onConfirm={handleSaveConfirm}
			/>
		</div>
	);
}
