"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Boxes,
	Cloud,
	Database,
	Loader2,
	PanelLeftClose,
	PanelLeftOpen,
	Play,
	Save,
	Sparkles,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type ImperativePanelHandle, Panel } from "react-resizable-panels";
import { useInvoke } from "../../../../hooks/use-invoke";
import { cn } from "../../../../lib/utils";
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
	type SqlCatalogTable,
	SqlEditor,
	extractParams,
	extractReferencedTables,
} from "../../../ui/sql-editor";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../../ui/tooltip";
import {
	OntologyActionParameterForm,
	humanizeIdentifier,
} from "../data-studio-panels";
import { EditorFooterBar, type LastRun } from "./editor-footer-bar";
import { QueryResultView } from "./query-result-view";
import { SaveQueryDialog } from "./save-query-dialog";
import { SavedQuerySidebar } from "./saved-query-sidebar";

const DEFAULT_LIMIT = 1000;

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
	const savedQueriesQuery = useInvoke(
		backend.queryState.listSavedQueries,
		backend.queryState,
		[appId, userScoped],
	);
	const savedQueries = savedQueriesQuery.data ?? [];

	const [sql, setSql] = useState("SELECT 1");
	const [surface, setSurface] = useState<WorkbenchSurfaceKind>("native");
	const [overlayId, setOverlayId] = useState<string | undefined>(undefined);
	const [remoteImportId, setRemoteImportId] = useState<string | undefined>(
		undefined,
	);
	const [paramValues, setParamValues] = useState<Record<string, unknown>>({});
	const [paramsValid, setParamsValid] = useState(true);
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

	const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
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
		(table: string) => `${userScoped ? "user" : "project"}:${table}`,
		[userScoped],
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
		if (surface !== "native") return;
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
		if (!sql.trim()) return;
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
			setResult(value);
			setLastRun({
				durationMs: performance.now() - startedAt,
				rowCount: value.row_count,
				ok: true,
			});
			setLiveMessage(`${value.row_count} rows returned`);
		} catch (error) {
			setRunError(
				error instanceof Error ? error.message : t('theQueryCouldNotRun', 'The query could not run.'),
			);
			setResult(null);
			setLastRun({
				durationMs: performance.now() - startedAt,
				rowCount: 0,
				ok: false,
			});
			setLiveMessage("Query failed");
		} finally {
			setRunning(false);
		}
	}, [
		appId,
		backend.queryState,
		backend.graphState,
		limit,
		overlayId,
		remoteImportId,
		paramValues,
		sql,
		surface,
		userScoped,
	]);

	const loadSavedQuery = useCallback((query: SavedQuery) => {
		setEditing(query);
		setSql(query.sql);
		setSurface(query.surface);
		setOverlayId(query.overlay_id);
		setLoadedParamSchema(query.param_schema);
		setParamValues({});
		setVizConfig(query.viz_config ?? { view: "table" });
		setLimit(query.default_limit ?? DEFAULT_LIMIT);
		setResult(null);
		setRunError(null);
		setLastRun(null);
	}, []);

	const startNewQuery = useCallback(() => {
		setEditing(null);
		setSql("SELECT 1");
		setLoadedParamSchema(undefined);
		setParamValues({});
		setVizConfig({ view: "table" });
		setLimit(DEFAULT_LIMIT);
		setResult(null);
		setRunError(null);
		setLastRun(null);
	}, []);
	const savedQueryScopeRef = useRef(userScoped);
	useEffect(() => {
		if (savedQueryScopeRef.current === userScoped) return;
		savedQueryScopeRef.current = userScoped;
		startNewQuery();
	}, [startNewQuery, userScoped]);
	useEffect(() => {
		if (surface !== "overlay") return;
		if (overlayId && ontologies.some((overlay) => overlay.id === overlayId)) {
			return;
		}
		setOverlayId(ontologies[0]?.id);
	}, [ontologies, overlayId, surface]);

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
			if (surface === "remote") return;
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
					error instanceof Error ? error.message : t('couldNotSaveTheQuery', 'Could not save the query.'),
				);
			} finally {
				setSaveBusy(false);
			}
		},
		[
			appId,
			backend.queryState,
			editing,
			limit,
			overlayId,
			paramNames.length,
			paramSchema,
			savedQueriesQuery,
			sql,
			surface,
			userScoped,
			vizConfig,
		],
	);

	const disabledReason = running
		? "Running…"
		: !sql.trim()
			? t('writeAQueryFirst', 'Write a query first')
			: surface === "overlay" && !overlayId
				? t('selectAnOntology', 'Select an ontology')
				: surface === "remote" && !remoteImportId
					? t('selectARemoteOntology', 'Select a remote ontology')
					: paramNames.length > 0 && !paramsValid
						? t('fixParameterValues', 'Fix parameter values')
						: null;
	const runDisabled = disabledReason !== null;

	const openSave = useCallback(() => {
		if (!sql.trim() || surface === "remote") return;
		setSaveError(null);
		setSaveOpen(true);
	}, [sql, surface]);

	const toggleSidebar = useCallback(() => {
		const panel = sidebarPanelRef.current;
		if (!panel) return;
		if (panel.isCollapsed()) panel.expand();
		else panel.collapse();
	}, []);

	const focusParamForm = useCallback(() => {
		const el = paramFormRef.current?.querySelector<HTMLElement>(
			t('inputSelectTextareaButton', 'input, select, textarea, button'),
		);
		el?.focus();
		el?.scrollIntoView({ block: "nearest" });
	}, []);

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

	return (
		<div
			className="flex h-full min-h-0 flex-col overflow-hidden"
			onKeyDown={handleKeyDown}
		>
			<output className="sr-only" aria-live="polite">
				{liveMessage}
			</output>

			<ResizablePanelGroup
				direction="horizontal"
				autoSaveId="qw-h"
				className="min-h-0 flex-1"
			>
				<Panel
					id="qw-sidebar"
					order={1}
					ref={sidebarPanelRef}
					collapsible
					collapsedSize={0}
					defaultSize={18}
					minSize={14}
					maxSize={30}
					onCollapse={() => setSidebarCollapsed(true)}
					onExpand={() => setSidebarCollapsed(false)}
					className="min-w-0"
				>
					{!sidebarCollapsed && (
						<SavedQuerySidebar
							queries={savedQueries}
							activeId={editing?.id}
							loading={savedQueriesQuery.isLoading}
							onSelect={loadSavedQuery}
							onNew={startNewQuery}
							onDelete={deleteSavedQuery}
							onDuplicate={async (query) => {
								await backend.queryState.createSavedQuery(
									appId,
									{
										name: t('nameCopy2', '{{name}} copy', { name: query.name }),
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
							}}
						/>
					)}
				</Panel>

				<ResizableHandle withHandle aria-label={t('resizeSavedqueriesSidebar', 'Resize saved-queries sidebar')} />

				<Panel id="qw-main" order={2} minSize={40} className="min-w-0">
					<section className="flex h-full min-h-0 min-w-0 flex-col">
						<div className="flex min-h-12 flex-wrap items-center gap-2 border-b bg-muted/20 px-3 py-1.5">
							<Button
								variant="ghost"
								size="icon"
								className="h-8 w-8 shrink-0"
								onClick={toggleSidebar}
								aria-expanded={!sidebarCollapsed}
								aria-controls="qw-sidebar"
								aria-label={
									sidebarCollapsed ? t('showSavedQueries', 'Show saved queries') : t('hideSavedQueries', 'Hide saved queries')
								}
							>
								{sidebarCollapsed ? (
									<PanelLeftOpen className="h-4 w-4" />
								) : (
									<PanelLeftClose className="h-4 w-4" />
								)}
							</Button>

							<div className="flex items-center gap-0.5 rounded-lg bg-muted p-0.5">
								<button
									type="button"
									aria-pressed={surface === "native"}
									aria-label={t('queryNativeTables', 'Query native tables')}
									onClick={() => setSurface("native")}
									className={cn(
										"flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
										surface === "native"
											? "bg-background text-foreground shadow-sm"
											: "text-muted-foreground hover:text-foreground",
									)}
								>
									<Database className="h-3.5 w-3.5" /> {t('native', 'Native')}
								</button>
								<button
									type="button"
									aria-pressed={surface === "overlay"}
									aria-label={t('queryOntology', 'Query ontology')}
									disabled={ontologies.length === 0}
									onClick={() => {
										setSurface("overlay");
										if (!overlayId) setOverlayId(ontologies[0]?.id);
									}}
									className={cn(
										"flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
										surface === "overlay"
											? "bg-background text-foreground shadow-sm"
											: "text-muted-foreground hover:text-foreground",
									)}
								>
									<Boxes className="h-3.5 w-3.5" /> {t('ontology', 'Ontology')}
								</button>
								<button
									type="button"
									aria-pressed={surface === "remote"}
									aria-label={t('queryRemoteOntology', 'Query remote ontology')}
									disabled={remoteImports.length === 0}
									onClick={() => {
										setSurface("remote");
										if (!remoteImportId)
											setRemoteImportId(remoteImports[0]?.id);
									}}
									className={cn(
										"flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
										surface === "remote"
											? "bg-background text-foreground shadow-sm"
											: "text-muted-foreground hover:text-foreground",
									)}
								>
									<Cloud className="h-3.5 w-3.5" /> {t('remote', 'Remote')}
								</button>
							</div>

							{surface !== "remote" && (
								<Select
									value={userScoped ? "user" : "project"}
									onValueChange={(value) => onScopeChange(value === "user")}
								>
									<SelectTrigger
										className="h-8 w-32"
										aria-label={t('databaseScope', 'Database scope')}
									>
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="project">{t('project', 'Project')}</SelectItem>
										<SelectItem value="user">{t('personal', 'Personal')}</SelectItem>
									</SelectContent>
								</Select>
							)}

							{surface === "overlay" && (
								<Select value={overlayId} onValueChange={setOverlayId}>
									<SelectTrigger className="h-8 w-44" aria-label="Ontology">
										<SelectValue placeholder={t('selectOntology', 'Select ontology')} />
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
											aria-label={t('remoteOntology', 'Remote ontology')}
										>
											<SelectValue placeholder={t('selectRemoteOntology', 'Select remote ontology')} />
										</SelectTrigger>
										<SelectContent>
											{remoteImports.map((imported) => (
												<SelectItem key={imported.id} value={imported.id}>
													{imported.contract.name}
													{` · `}
													{resolveSourceName?.(imported.target_app_id) ??
														imported.target_app_id}
												</SelectItem>
											))}
										</SelectContent>
									</Select>
									<Badge variant="outline" className="hidden gap-1 sm:flex">
										<Cloud className="h-3 w-3" /> {t('readonly', 'Read-only')}
									</Badge>
								</>
							)}

							<div className="ml-auto flex items-center gap-2">
								{editing && (
									<Badge variant="secondary" className="gap-1 text-xs">
										<Sparkles className="h-3 w-3" />
										<span className="max-w-40 truncate">{editing.name}</span>
									</Badge>
								)}
								<Button
									variant="outline"
									size="sm"
									className="h-8 gap-1.5"
									disabled={!sql.trim() || surface === "remote"}
									title={
										surface === "remote"
											? t('remoteQueriesAreReadonlyPreviewsAndCannotBeSaved', 'Remote queries are read-only previews and cannot be saved')
											: undefined
									}
									onClick={openSave}
								>
									<Save className="h-4 w-4" /> {t('save', 'Save')}
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
												)}<Trans i18nKey="runKbdClassnameml05HiddenRoundedBorderBorderprimaryforeground30Bgprimaryforeground10Px1Text10pxFontmediumSminlineKbd">Run
												<kbd className="ml-0.5 hidden rounded border border-primary-foreground/30 bg-primary-foreground/10 px-1 text-[10px] font-medium sm:inline">
													⌘↵
												</kbd></Trans></Button>
										</span>
									</TooltipTrigger>
									<TooltipContent>
										{disabledReason ?? "Run query"}
									</TooltipContent>
								</Tooltip>
							</div>
						</div>

						<ResizablePanelGroup
							direction="vertical"
							autoSaveId="qw-v"
							className="min-h-0 flex-1"
						>
							<Panel id="qw-editor" order={1} defaultSize={45} minSize={20}>
								<div className="flex h-full min-h-0 flex-col">
									<div className="flex min-h-0 flex-1">
										<div className="min-h-0 min-w-0 flex-1">
											<SqlEditor
												value={sql}
												onChange={setSql}
												catalog={catalog}
												onRun={() => void runQuery()}
												onCursorChange={setCursor}
											/>
										</div>
										{paramNames.length > 0 && (
											<div
												ref={paramFormRef}
												className="w-72 shrink-0 overflow-y-auto border-l p-3"
											>
												<OntologyActionParameterForm
													actionId="workbench-query"
													schema={paramSchema}
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
										lastRun={lastRun}
										onParamClick={focusParamForm}
									/>
								</div>
							</Panel>

							<ResizableHandle
								withHandle
								aria-label={t('resizeEditorAndResults', 'Resize editor and results')}
							/>

							<Panel id="qw-results" order={2} minSize={20}>
								<QueryResultView
									result={result}
									loading={running}
									error={runError}
									vizConfig={vizConfig}
									onVizConfigChange={setVizConfig}
									onRun={() => void runQuery()}
								/>
							</Panel>
						</ResizablePanelGroup>
					</section>
				</Panel>
			</ResizablePanelGroup>

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
