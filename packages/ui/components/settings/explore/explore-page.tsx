"use client";

import {
	Badge,
	Button,
	Card,
	IIndexType,
	Input,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	cn,
	useAssistantSurface,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import {
	DataStudioOverview,
	ObjectExplorerPanel,
	OntologyActionsPanel,
	OntologyModelPanel,
	OntologySharingPanel,
} from "@flow-like/flow-like-ui/components/settings/data-studio/data-studio-panels";
import {
	type DataStudioTableInfo,
	OntologySetupDialog,
} from "@flow-like/flow-like-ui/components/settings/data-studio/ontology-setup-dialog";
import { QueryWorkbench } from "@flow-like/flow-like-ui/components/settings/data-studio/query-workbench";
import { TableDesignerDialog } from "@flow-like/flow-like-ui/components/settings/data-studio/table-designer-dialog";
import { OntologyExplorer } from "@flow-like/flow-like-ui/components/ui/graph";
import LanceDBExplorer from "@flow-like/flow-like-ui/components/ui/lance-viewer";
import type {
	CreateOverlayPayload,
	EdgeLabelMapping,
	GraphOverlay,
} from "@flow-like/flow-like-ui/state/backend-state/graph-state";
import {
	AlertTriangle,
	ArrowDownAZ,
	ArrowLeftIcon,
	ArrowUpAZ,
	Box,
	Cloud,
	Database,
	Globe,
	Layers3,
	LayoutDashboard,
	Network,
	Plus,
	RefreshCw,
	Search,
	Share2,
	SquareTerminal,
	User,
	Workflow,
	X,
} from "lucide-react";
import {
	type ReadonlyURLSearchParams,
	usePathname,
	useRouter,
	useSearchParams,
} from "next/navigation";
import type React from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

export interface ExploreDataPageProps {
	appId: string;
}

const DEFAULT_TABLE_PAGE_SIZE = 25;

function extractErrorMessage(err: unknown): string {
	if (err instanceof Error) return err.message;
	if (typeof err === "string") return err;
	if (err && typeof err === "object") {
		const obj = err as Record<string, unknown>;
		if (typeof obj.error === "string") return obj.error;
		if (typeof obj.message === "string") return obj.message;
		try {
			return JSON.stringify(err);
		} catch {
			return String(err);
		}
	}
	return String(err);
}

export const ExploreDataPage: React.FC<ExploreDataPageProps> = ({ appId }) => {
	const router = useRouter();
	const searchParams = useSearchParams();
	const tableParam = searchParams?.get("table") ?? null;
	const overlayParam = searchParams?.get("overlay") ?? null;
	const pathname = usePathname();

	const table = useMemo(() => {
		if (!tableParam) return "";
		try {
			return decodeURIComponent(tableParam);
		} catch {
			return tableParam;
		}
	}, [tableParam]);

	const userScoped = searchParams?.get("scope") === "user";

	// Publish the open Data Studio page so the global assistant defaults data questions to this
	// app/overlay (via data_studio_agent) without asking which project. Cleared on unmount.
	const setDataStudioSurface = useAssistantSurface(
		(state) => state.setDataStudioSurface,
	);
	useEffect(() => {
		setDataStudioSurface({
			appId,
			overlayId: overlayParam ?? undefined,
			selectedTable: table || undefined,
		});
		return () => setDataStudioSurface(null);
	}, [appId, overlayParam, table, setDataStudioSurface]);

	if (overlayParam) {
		return (
			<OverlayView
				appId={appId}
				overlayId={overlayParam}
				onBack={() => {
					const params = new URLSearchParams(searchParams?.toString() ?? "");
					params.delete("overlay");
					router.push(`${pathname}?${params.toString()}`);
				}}
			/>
		);
	}

	return table ? (
		<TableView
			table={table}
			appId={appId}
			userScoped={userScoped}
			onBack={() => {
				const params = new URLSearchParams(searchParams?.toString() ?? "");
				params.delete("table");
				params.delete("scope");
				router.push(`${pathname}?${params.toString()}`);
			}}
		/>
	) : (
		<DatabaseOverview appId={appId} searchParams={searchParams} />
	);
};

export const DataStudioPage = ExploreDataPage;

function TableView({
	table,
	appId,
	userScoped,
	onBack,
}: Readonly<{
	table: string;
	appId: string;
	userScoped?: boolean;
	onBack: () => void;
}>) {
	const backend = useBackend();
	const router = useRouter();
	const pathname = usePathname();
	const searchParams = useSearchParams();

	const pageParam = searchParams?.get("page");
	const pageSizeParam = searchParams?.get("pageSize");
	const page = pageParam ? Math.max(1, Number.parseInt(pageParam, 10) || 1) : 1;
	const pageSize = pageSizeParam
		? Number.parseInt(pageSizeParam, 10) || DEFAULT_TABLE_PAGE_SIZE
		: DEFAULT_TABLE_PAGE_SIZE;
	const offset = (page - 1) * pageSize;

	const schema = useInvoke(backend.dbState.getSchema, backend.dbState, [
		appId,
		table,
		userScoped,
	]);
	const count = useInvoke(backend.dbState.countItems, backend.dbState, [
		appId,
		table,
		userScoped,
	]);
	const list = useInvoke(backend.dbState.listItems, backend.dbState, [
		appId,
		table,
		offset,
		pageSize,
		userScoped,
	]);

	// The explorer reports its pagination once on mount, which for a deep link is
	// already what the URL says. Navigating anyway costs a client-side transition
	// on every mount for no state change, so only write when something differs.
	const updateUrlParams = useCallback(
		(newPage: number, newPageSize: number) => {
			const current = searchParams?.toString() ?? "";
			const params = new URLSearchParams(current);
			if (newPage > 1) {
				params.set("page", String(newPage));
			} else {
				params.delete("page");
			}
			if (newPageSize !== DEFAULT_TABLE_PAGE_SIZE) {
				params.set("pageSize", String(newPageSize));
			} else {
				params.delete("pageSize");
			}
			const next = params.toString();
			if (next === current) return;
			router.replace(`${pathname}?${next}`, { scroll: false });
		},
		[router, pathname, searchParams],
	);

	const handleRefresh = useCallback(() => {
		schema.refetch();
		count.refetch();
		list.refetch();
	}, [schema, count, list]);

	const handleOptimize = useCallback(async () => {
		try {
			await backend.dbState.optimize(appId, table, undefined, userScoped);
			toast.success("Optimized table");
			handleRefresh();
		} catch (err) {
			toast.error(`Optimize failed: ${extractErrorMessage(err)}`);
			throw err;
		}
	}, [backend.dbState, appId, table, userScoped, handleRefresh]);

	const handleUpdateItem = useCallback(
		async (filter: string, updates: Record<string, unknown>) => {
			try {
				await backend.dbState.updateItem(
					appId,
					table,
					filter,
					updates,
					userScoped,
				);
				handleRefresh();
			} catch (err) {
				toast.error(`Update failed: ${extractErrorMessage(err)}`);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	const handleDropColumns = useCallback(
		async (columns: string[]) => {
			try {
				await backend.dbState.dropColumns(appId, table, columns, userScoped);
				toast.success(`Dropped column${columns.length > 1 ? "s" : ""}`);
				handleRefresh();
			} catch (err) {
				toast.error(`Drop column failed: ${extractErrorMessage(err)}`);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	const handleAddColumn = useCallback(
		async (name: string, sqlExpression: string) => {
			try {
				await backend.dbState.addColumn(
					appId,
					table,
					{
						name,
						sql_expression: sqlExpression,
					},
					userScoped,
				);
				toast.success(`Added column "${name}"`);
				handleRefresh();
			} catch (err) {
				toast.error(`Add column failed: ${extractErrorMessage(err)}`);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	const handleAlterColumn = useCallback(
		async (column: string, nullable: boolean) => {
			try {
				await backend.dbState.alterColumn(
					appId,
					table,
					column,
					nullable,
					userScoped,
				);
				toast.success(`Altered column "${column}"`);
				handleRefresh();
			} catch (err) {
				toast.error(`Alter column failed: ${extractErrorMessage(err)}`);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	const handleGetIndices = useCallback(async () => {
		return backend.dbState.getIndices(appId, table, userScoped);
	}, [backend.dbState, appId, table, userScoped]);

	const handleDropIndex = useCallback(
		async (indexName: string) => {
			try {
				await backend.dbState.dropIndex(appId, table, indexName, userScoped);
				toast.success(`Dropped index "${indexName}"`);
				handleRefresh();
			} catch (err) {
				toast.error(`Drop index failed: ${extractErrorMessage(err)}`);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	const handleBuildIndex = useCallback(
		async (column: string, indexType: string) => {
			const typeMap: Record<string, IIndexType> = {
				fulltext: IIndexType.FullText,
				btree: IIndexType.BTree,
				bitmap: IIndexType.Bitmap,
				labellist: IIndexType.LabelList,
				auto: IIndexType.Auto,
			};
			const enumType = typeMap[indexType.toLowerCase()] ?? IIndexType.Auto;
			try {
				await backend.dbState.buildIndex(
					appId,
					table,
					column,
					enumType,
					undefined,
					userScoped,
				);
				toast.success(`Built index on "${column}"`);
				handleRefresh();
			} catch (err) {
				toast.error(`Build index failed: ${extractErrorMessage(err)}`);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	if (!schema.data || !list.data) {
		return <TableViewLoadingState onBack={onBack} tableName={table} />;
	}

	return (
		<div className="flex flex-col h-full grow max-h-full min-w-0">
			<LanceDBExplorer
				total={count.data}
				tableName={table}
				arrowSchema={schema.data}
				rows={list.data}
				initialPage={page}
				initialPageSize={pageSize}
				onPageRequest={(args) => {
					updateUrlParams(args.page, args.pageSize);
				}}
				loading={list.isLoading}
				error={list.error?.message}
				onRefresh={handleRefresh}
				onOptimize={handleOptimize}
				onUpdateItem={handleUpdateItem}
				onDropColumns={handleDropColumns}
				onAddColumn={handleAddColumn}
				onAlterColumn={handleAlterColumn}
				onGetIndices={handleGetIndices}
				onDropIndex={handleDropIndex}
				onBuildIndex={handleBuildIndex}
			>
				<Button variant={"default"} size={"sm"} onClick={onBack}>
					<ArrowLeftIcon />
					Back
				</Button>
			</LanceDBExplorer>
		</div>
	);
}

interface DatabaseOverviewProps {
	appId: string;
	searchParams: ReadonlyURLSearchParams;
}

const DATA_STUDIO_VIEWS = [
	"overview",
	"objects",
	"model",
	"actions",
	"sharing",
	"sources",
	"queries",
] as const;

type DataStudioView = (typeof DATA_STUDIO_VIEWS)[number];

function isDataStudioView(view: string | null): view is DataStudioView {
	return DATA_STUDIO_VIEWS.includes(view as DataStudioView);
}

interface Table {
	name: string;
	rowCount?: number;
	userScoped?: boolean;
}

const DatabaseOverview: React.FC<DatabaseOverviewProps> = ({
	appId,
	searchParams,
}) => {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const router = useRouter();
	const pathname = usePathname();
	const requestedView = searchParams.get("view");
	const urlView: DataStudioView = isDataStudioView(requestedView)
		? requestedView
		: "overview";
	const [activeView, setActiveViewState] = useState<DataStudioView>(urlView);

	// Keep back/forward navigation and copied deep links authoritative. Tab clicks
	// update this same state synchronously below so the controlled Radix tabs do
	// not snap back while Next is still publishing the new search parameters.
	useEffect(() => {
		setActiveViewState(urlView);
	}, [urlView]);
	const [actionBoardsRequested, setActionBoardsRequested] = useState(false);
	const tables = useInvoke(backend.dbState.listTables, backend.dbState, [
		appId,
	]);
	const userTables = useInvoke(
		backend.dbState.listTablesUser,
		backend.dbState,
		[appId],
	);
	const ontologies = useInvoke(
		backend.graphState.listOverlays,
		backend.graphState,
		[appId],
	);
	const userOntologies = useInvoke(
		backend.graphState.listOverlays,
		backend.graphState,
		[appId, true],
		activeView === "queries",
	);
	const boards = useInvoke(
		backend.boardState.getBoards,
		backend.boardState,
		[appId],
		activeView === "actions" && actionBoardsRequested,
	);
	// Remote/installed ontologies are a first-class data source: they show up as
	// objects, sources, and a query surface — not just in the sharing/model tabs.
	const remoteDataNeeded = activeView !== "actions";
	const appConnections = useInvoke(
		backend.teamState.getAppConnections,
		backend.teamState,
		[appId],
		remoteDataNeeded,
	);
	const installedOntologies = useInvoke(
		backend.graphState.listRemoteOntologyImports,
		backend.graphState,
		[appId],
		remoteDataNeeded,
	);

	const [query, setQuery] = useState<string>("");
	const [sortAsc, setSortAsc] = useState<boolean>(true);
	const [setupOpen, setSetupOpen] = useState(false);
	const [designerOpen, setDesignerOpen] = useState(false);
	const processedTables = useMemo(() => {
		const projectTables = (tables.data ?? []).map((name): Table => ({ name }));
		const userScopedTables = (userTables.data ?? []).map(
			(name): Table => ({ name, userScoped: true }),
		);
		return [...projectTables, ...userScopedTables];
	}, [tables.data, userTables.data]);

	const filteredAndSortedTables = useMemo(() => {
		const collator = new Intl.Collator(undefined, {
			numeric: true,
			sensitivity: "base",
		});

		const queryLower = query.trim().toLowerCase();

		return processedTables
			.filter(
				(table) => !queryLower || table.name.toLowerCase().includes(queryLower),
			)
			.sort((a, b) =>
				sortAsc
					? collator.compare(a.name, b.name)
					: collator.compare(b.name, a.name),
			);
	}, [processedTables, query, sortAsc]);

	const navigateToTable = useCallback(
		(tableName: string, userScoped?: boolean) => {
			const params = new URLSearchParams(searchParams?.toString() ?? "");
			params.set("table", tableName);
			if (userScoped) {
				params.set("scope", "user");
			}
			router.push(`${pathname}?${params.toString()}`);
		},
		[router, pathname, searchParams],
	);

	const refreshStudio = useCallback(() => {
		tables.refetch();
		userTables.refetch();
		ontologies.refetch();
		if (activeView === "actions") boards.refetch();
		if (remoteDataNeeded) {
			appConnections.refetch();
			installedOntologies.refetch();
		}
	}, [
		tables.refetch,
		userTables.refetch,
		ontologies.refetch,
		boards.refetch,
		appConnections.refetch,
		installedOntologies.refetch,
		activeView,
		remoteDataNeeded,
	]);

	const openRemoteSource = useCallback(
		(importId: string) => {
			const params = new URLSearchParams(searchParams?.toString() ?? "");
			params.set("view", "objects");
			params.set("source", importId);
			router.replace(`${pathname}?${params.toString()}`, { scroll: false });
		},
		[router, pathname, searchParams],
	);

	const navigateToOntology = useCallback(
		(ontologyId: string) => {
			const params = new URLSearchParams(searchParams?.toString() ?? "");
			params.set("overlay", ontologyId);
			router.push(`${pathname}?${params.toString()}`);
		},
		[router, pathname, searchParams],
	);

	const setActiveView = useCallback(
		(view: string) => {
			if (!isDataStudioView(view)) return;
			setActiveViewState(view);
			const params = new URLSearchParams(searchParams?.toString() ?? "");
			if (view === "overview") params.delete("view");
			else params.set("view", view);
			router.replace(`${pathname}?${params.toString()}`, { scroll: false });
		},
		[router, pathname, searchParams],
	);

	const setQueryScope = useCallback(
		(userScoped: boolean) => {
			const params = new URLSearchParams(searchParams?.toString() ?? "");
			params.set("view", "queries");
			if (userScoped) params.set("scope", "user");
			else params.delete("scope");
			router.replace(`${pathname}?${params.toString()}`, { scroll: false });
		},
		[router, pathname, searchParams],
	);

	const createOntology = useCallback(
		async (payload: CreateOverlayPayload) => {
			await backend.graphState.createOverlay(appId, payload);
			await ontologies.refetch();
			await invalidate(backend.boardState.getCatalog, [appId]);
			toast.success(`Created ${payload.name}`);
		},
		[appId, backend.graphState, backend.boardState, invalidate, ontologies],
	);

	const saveActions = useCallback(
		async (
			ontologyId: string,
			actions: NonNullable<GraphOverlay["actions"]>,
		) => {
			const ontology = ontologies.data?.find(
				(candidate) => candidate.id === ontologyId,
			);
			await backend.graphState.updateOverlay(appId, ontologyId, {
				expected_updated_at: ontology?.updated_at,
				actions,
			});
			await ontologies.refetch();
			await invalidate(backend.boardState.getCatalog, [appId]);
			toast.success("Action binding saved");
		},
		[appId, backend.graphState, backend.boardState, invalidate, ontologies],
	);

	const saveEdges = useCallback(
		async (ontologyId: string, edges: EdgeLabelMapping[]) => {
			// Relationship controls can enqueue several edits in quick succession.
			// Resolve the current concurrency token for each serialized write instead
			// of capturing the ontology list from the render that started the queue.
			const ontology = await backend.graphState.getOverlay(appId, ontologyId);
			await backend.graphState.updateOverlay(appId, ontologyId, {
				expected_updated_at: ontology.updated_at,
				edges,
			});
			// Once the mutation succeeds, a transient refresh failure must not make
			// the editor roll back a relationship that is already persisted.
			await Promise.allSettled([
				ontologies.refetch(),
				invalidate(backend.boardState.getCatalog, [appId]),
			]);
			toast.success("Ontology model saved");
		},
		[appId, backend.graphState, backend.boardState, invalidate, ontologies],
	);

	const updateSharing = useCallback(
		async (
			ontologyId: string,
			patch: Partial<Pick<GraphOverlay, "exposed" | "bindings_enabled">>,
		) => {
			const ontology = ontologies.data?.find(
				(candidate) => candidate.id === ontologyId,
			);
			await backend.graphState.updateOverlay(appId, ontologyId, {
				expected_updated_at: ontology?.updated_at,
				...patch,
			});
			await ontologies.refetch();
			await invalidate(backend.boardState.getCatalog, [appId]);
		},
		[appId, backend.graphState, backend.boardState, invalidate, ontologies],
	);

	const sampleObjects = useCallback(
		(ontologyId: string, objectType: string, limit: number) =>
			backend.graphState.sample(appId, ontologyId, objectType, limit),
		[appId, backend.graphState],
	);

	const sampleRemoteObjects = useCallback(
		(importId: string, objectType: string, limit: number) =>
			backend.graphState.sampleRemoteImport(appId, importId, objectType, limit),
		[appId, backend.graphState],
	);

	const invokeOntologyAction = useCallback(
		async (
			ontologyId: string,
			actionId: string,
			payload: Parameters<typeof backend.graphState.invokeOntologyAction>[3],
			onStatus?: Parameters<typeof backend.graphState.invokeOntologyAction>[4],
		) => {
			let governedPayload = payload;
			const isOffline = await backend.isOffline(appId);
			const action = ontologies.data
				?.find((ontology) => ontology.id === ontologyId)
				?.actions?.find((candidate) => candidate.id === actionId);

			if (!isOffline && backend.eventState.checkOAuthRequirements) {
				const prerun = await backend.graphState.prerunOntologyAction(
					appId,
					ontologyId,
					actionId,
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
				governedPayload = { ...payload, oauth_tokens: oauth.tokens };
			}

			return backend.graphState.invokeOntologyAction(
				appId,
				ontologyId,
				actionId,
				governedPayload,
				onStatus,
			);
		},
		[appId, backend, backend.eventState, backend.graphState, ontologies.data],
	);

	const loadRemoteOntologies = useCallback(
		(targetAppId: string) =>
			backend.graphState.listRemoteOntologies(appId, targetAppId),
		[appId, backend.graphState],
	);

	const installRemoteOntology = useCallback(
		async (targetAppId: string, ontologyId: string) => {
			await backend.graphState.installRemoteOntology(
				appId,
				targetAppId,
				ontologyId,
			);
			await installedOntologies.refetch();
			await invalidate(backend.boardState.getCatalog, [appId]);
			toast.success("Remote ontology bindings installed");
		},
		[
			appId,
			backend.boardState,
			backend.graphState,
			installedOntologies,
			invalidate,
		],
	);

	const uninstallRemoteOntology = useCallback(
		async (targetAppId: string, ontologyId: string) => {
			await backend.graphState.uninstallRemoteOntology(
				appId,
				targetAppId,
				ontologyId,
			);
			await installedOntologies.refetch();
			await invalidate(backend.boardState.getCatalog, [appId]);
			toast.success("Remote ontology bindings uninstalled");
		},
		[
			appId,
			backend.boardState,
			backend.graphState,
			installedOntologies,
			invalidate,
		],
	);

	const clearSearch = useCallback(() => {
		setQuery("");
	}, []);

	const toggleSort = useCallback(() => {
		setSortAsc((prev) => !prev);
	}, []);

	const isLoading =
		(tables.isLoading || userTables.isLoading || ontologies.isLoading) &&
		!processedTables.length &&
		!(ontologies.data?.length ?? 0);

	if (isLoading) {
		return <LoadingState />;
	}

	if (tables.error && userTables.error && ontologies.error) {
		return <ErrorState onRetry={refreshStudio} />;
	}

	const ontologyData = ontologies.data ?? [];
	const connections = [
		...(appConnections.data?.incoming ?? []),
		...(appConnections.data?.outgoing ?? []),
	];
	const installedData = installedOntologies.data ?? [];
	// Only imports with live bindings are usable as data sources; disabled ones
	// stay visible for management in the sharing/model tabs.
	const usableImports = installedData.filter(
		(imported) => imported.bindings_enabled,
	);
	const resolveSourceName = (targetAppId: string) =>
		connections.find(
			(connection) =>
				connection.target_app_id === targetAppId ||
				connection.source_app_id === targetAppId,
		)?.app_name ?? targetAppId;

	const failedQueries: { name: string; onRetry: () => void }[] = [];
	if (tables.error) {
		failedQueries.push({
			name: "project tables",
			onRetry: () => {
				tables.refetch();
			},
		});
	}
	if (userTables.error) {
		failedQueries.push({
			name: "user tables",
			onRetry: () => {
				userTables.refetch();
			},
		});
	}
	if (ontologies.error) {
		failedQueries.push({
			name: "ontologies",
			onRetry: () => {
				ontologies.refetch();
			},
		});
	}

	return (
		<div className="flex flex-col h-full">
			<div className="p-6 pb-0">
				<header className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
					<div className="flex items-center gap-3">
						<div className="rounded-xl bg-primary/10 p-2.5 text-primary">
							<Layers3 className="h-5 w-5" />
						</div>
						<div>
							<h1 className="text-2xl font-semibold">Data Studio</h1>
							<p className="text-sm text-muted-foreground">
								Model, explore, operate, and share your project data.
							</p>
						</div>
					</div>
					<div className="flex items-center gap-2">
						<Button variant="outline" size="sm" onClick={refreshStudio}>
							<RefreshCw className="h-4 w-4" /> Refresh
						</Button>
						<Button size="sm" onClick={() => setSetupOpen(true)}>
							<Plus className="h-4 w-4" /> Set up ontology
						</Button>
					</div>
				</header>
				{failedQueries.length > 0 && (
					<PartialFailureAlert failures={failedQueries} />
				)}
			</div>
			<Tabs
				value={activeView}
				onValueChange={setActiveView}
				className="flex flex-col flex-1 min-h-0"
			>
				<div className="overflow-x-auto px-6 pt-4">
					<TabsList className="w-max">
						<TabsTrigger value="overview">
							<LayoutDashboard className="mr-1.5 h-3.5 w-3.5" />
							Overview
						</TabsTrigger>
						<TabsTrigger value="objects">
							<Box className="mr-1.5 h-3.5 w-3.5" />
							Explore
						</TabsTrigger>
						<TabsTrigger value="model">
							<Network className="mr-1.5 h-3.5 w-3.5" />
							Model
						</TabsTrigger>
						<TabsTrigger value="actions">
							<Workflow className="mr-1.5 h-3.5 w-3.5" />
							Actions
						</TabsTrigger>
						<TabsTrigger value="sharing">
							<Share2 className="mr-1.5 h-3.5 w-3.5" />
							Sharing
						</TabsTrigger>
						<TabsTrigger value="sources">
							<Database className="mr-1.5 h-3.5 w-3.5" />
							Sources
						</TabsTrigger>
						<TabsTrigger value="queries">
							<SquareTerminal className="mr-1.5 h-3.5 w-3.5" />
							Queries
						</TabsTrigger>
					</TabsList>
				</div>
				<TabsContent value="overview" className="flex-1 overflow-y-auto p-6">
					<DataStudioOverview
						ontologies={ontologyData}
						tableCount={processedTables.length}
						remoteCount={installedData.length}
						onCreateOntology={() => setSetupOpen(true)}
						onOpenOntology={navigateToOntology}
						onNavigate={setActiveView}
					/>
				</TabsContent>
				<TabsContent value="objects" className="min-h-0 flex-1 p-6">
					<ObjectExplorerPanel
						ontologies={ontologyData}
						remoteImports={usableImports}
						initialSourceValue={searchParams.get("source") ?? undefined}
						onCreateOntology={() => setSetupOpen(true)}
						onSample={sampleObjects}
						onSampleRemote={sampleRemoteObjects}
						onInvokeAction={invokeOntologyAction}
						resolveSourceName={resolveSourceName}
					/>
				</TabsContent>
				<TabsContent value="model" className="flex-1 overflow-y-auto p-6">
					<OntologyModelPanel
						appId={appId}
						ontologies={ontologyData}
						installedOntologies={installedOntologies.data ?? []}
						onCreateOntology={() => setSetupOpen(true)}
						onOpenOntology={navigateToOntology}
						onSaveEdges={saveEdges}
					/>
				</TabsContent>
				<TabsContent value="actions" className="flex-1 overflow-y-auto p-6">
					<OntologyActionsPanel
						ontologies={ontologyData}
						boards={boards.data ?? []}
						appId={appId}
						onCreateOntology={() => setSetupOpen(true)}
						onNeedBoards={() => setActionBoardsRequested(true)}
						onSaveActions={saveActions}
					/>
				</TabsContent>
				<TabsContent value="sharing" className="flex-1 overflow-y-auto p-6">
					<OntologySharingPanel
						ontologies={ontologyData}
						connections={connections}
						remoteConnections={appConnections.data?.outgoing ?? []}
						installedOntologies={installedOntologies.data ?? []}
						installedOntologiesLoading={installedOntologies.isLoading}
						installedOntologiesError={installedOntologies.error?.message}
						onCreateOntology={() => setSetupOpen(true)}
						onUpdateOntology={updateSharing}
						onLoadRemoteOntologies={loadRemoteOntologies}
						onInstallRemoteOntology={installRemoteOntology}
						onUninstallRemoteOntology={uninstallRemoteOntology}
					/>
				</TabsContent>
				<TabsContent
					value="sources"
					className="flex-1 overflow-y-auto p-6 space-y-4"
				>
					<div className="flex items-center justify-between gap-3">
						<div>
							<h2 className="font-semibold">Native tables</h2>
							<p className="text-sm text-muted-foreground">
								Open a source to inspect rows, schema, and indexes.
							</p>
						</div>
						<div className="flex items-center gap-2">
							<Button
								variant="ghost"
								size="icon"
								onClick={toggleSort}
								title={`Sort ${sortAsc ? "descending" : "ascending"}`}
							>
								{sortAsc ? (
									<ArrowUpAZ className="h-4 w-4" />
								) : (
									<ArrowDownAZ className="h-4 w-4" />
								)}
							</Button>
							<Button size="sm" onClick={() => setDesignerOpen(true)}>
								<Plus className="h-4 w-4" /> New table
							</Button>
						</div>
					</div>
					<SearchInput
						value={query}
						onChange={setQuery}
						onClear={clearSearch}
					/>
					<TableGrid
						tables={filteredAndSortedTables}
						onSelectTable={navigateToTable}
						searchQuery={query}
						onCreate={() => setDesignerOpen(true)}
					/>
					{usableImports.length > 0 && (
						<div className="space-y-4 pt-4">
							<div>
								<h2 className="flex items-center gap-2 font-semibold">
									<Cloud className="h-4 w-4" /> Remote objects
								</h2>
								<p className="text-sm text-muted-foreground">
									Installed from connected projects. Read-only previews resolve
									live against the source.
								</p>
							</div>
							<div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
								{usableImports.map((imported) => (
									<RemoteSourceCard
										key={imported.id}
										name={imported.contract.name}
										sourceName={resolveSourceName(imported.target_app_id)}
										objectCount={imported.contract.nodes.length}
										onOpen={() => openRemoteSource(imported.id)}
									/>
								))}
							</div>
						</div>
					)}
				</TabsContent>
				<TabsContent value="queries" className="min-h-0 flex-1 p-0">
					<QueryWorkbench
						appId={appId}
						ontologies={
							searchParams.get("scope") === "user"
								? (userOntologies.data ?? [])
								: ontologyData
						}
						remoteImports={usableImports}
						resolveSourceName={resolveSourceName}
						projectTables={tables.data ?? []}
						userTables={userTables.data ?? []}
						userScoped={searchParams.get("scope") === "user"}
						onScopeChange={setQueryScope}
					/>
				</TabsContent>
			</Tabs>
			<OntologySetupDialog
				open={setupOpen}
				onOpenChange={setSetupOpen}
				appId={appId}
				tables={processedTables as DataStudioTableInfo[]}
				loadSchema={(table) =>
					backend.dbState.getSchema(appId, table.name, table.userScoped)
				}
				onCreate={createOntology}
			/>
			<TableDesignerDialog
				open={designerOpen}
				onOpenChange={setDesignerOpen}
				appId={appId}
				existingTables={processedTables.map((table) => table.name)}
				onCreated={(name, userScoped) => {
					tables.refetch();
					userTables.refetch();
					navigateToTable(name, userScoped);
				}}
			/>
		</div>
	);
};

interface SearchInputProps {
	value: string;
	onChange: (value: string) => void;
	onClear: () => void;
}

const SearchInput: React.FC<SearchInputProps> = ({
	value,
	onChange,
	onClear,
}) => (
	<div className="relative max-w-xl">
		<Search className="absolute left-3 top-2.5 h-4 w-4 text-muted-foreground pointer-events-none" />
		<Input
			value={value}
			onChange={(event) => onChange(event.target.value)}
			placeholder="Search tables..."
			className="pl-9 pr-9"
		/>
		{value && (
			<Button
				variant="ghost"
				size="sm"
				onClick={onClear}
				className="absolute right-1 top-1 h-8 w-8 p-0"
				title="Clear search"
			>
				<X className="h-4 w-4" />
			</Button>
		)}
	</div>
);

interface TableGridProps {
	tables: Table[];
	onSelectTable: (tableName: string, userScoped?: boolean) => void;
	searchQuery: string;
	onCreate: () => void;
}

const TableGrid: React.FC<TableGridProps> = ({
	tables,
	onSelectTable,
	searchQuery,
	onCreate,
}) => {
	if (!tables.length && searchQuery) {
		return (
			<div className="rounded-lg border bg-card p-8 text-center">
				<Search className="mx-auto h-10 w-10 text-muted-foreground mb-4" />
				<h3 className="text-lg font-semibold mb-2">No matches found</h3>
				<p className="text-sm text-muted-foreground">
					No tables match &quot;
					<span className="font-medium">{searchQuery}</span>&quot;.
				</p>
			</div>
		);
	}

	if (!tables.length) {
		return (
			<div className="flex flex-col items-center justify-center rounded-lg border border-dashed bg-muted/20 p-10 text-center">
				<div className="mb-4 rounded-2xl bg-primary/10 p-3 text-primary">
					<Database className="h-6 w-6" />
				</div>
				<h3 className="font-semibold">No tables yet</h3>
				<p className="mt-1 max-w-sm text-sm text-muted-foreground">
					Create a native table to store structured data, then explore rows,
					schema, and indexes.
				</p>
				<Button className="mt-5" onClick={onCreate}>
					<Plus className="h-4 w-4" /> New table
				</Button>
			</div>
		);
	}

	return (
		<div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-4">
			{tables.map((table) => (
				<TableCard
					key={`${table.userScoped ? "user:" : ""}${table.name}`}
					table={table}
					onSelect={() => onSelectTable(table.name, table.userScoped)}
				/>
			))}
		</div>
	);
};

interface TableCardProps {
	table: Table;
	onSelect: () => void;
}

const TableCard: React.FC<TableCardProps> = ({ table, onSelect }) => {
	return (
		<Card className="group cursor-pointer transition-all duration-200 hover:shadow-lg hover:bg-accent/50 border overflow-hidden">
			<button
				type="button"
				onClick={onSelect}
				className="w-full h-full p-0 text-left"
				title={`Open table: ${table.name}`}
			>
				<div className="p-5 space-y-5">
					<div className="flex items-start justify-between gap-3">
						<div className="flex items-center gap-3 min-w-0">
							<div
								className={cn(
									"shrink-0 rounded-xl p-2.5 transition-colors",
									table.userScoped
										? "bg-amber-500/10 group-hover:bg-amber-500/20"
										: "bg-primary/10 group-hover:bg-primary/20",
								)}
							>
								<Database
									className={cn(
										"h-5 w-5",
										table.userScoped ? "text-amber-500" : "text-primary",
									)}
								/>
							</div>
							<div className="min-w-0">
								<h3 className="font-semibold text-sm leading-tight truncate">
									{table.name}
								</h3>
							</div>
						</div>
						{table.userScoped ? (
							<Badge
								variant="outline"
								className="shrink-0 bg-amber-500/10 text-amber-500 border-amber-500/20 text-[10px] gap-1"
							>
								<User className="h-3 w-3" />
								User scoped
							</Badge>
						) : (
							<Badge
								variant="outline"
								className="shrink-0 bg-primary/10 text-primary border-primary/20 text-[10px] gap-1"
							>
								<Globe className="h-3 w-3" />
								Shared
							</Badge>
						)}
					</div>

					<div className="flex items-center justify-between border-t pt-3 text-xs text-muted-foreground">
						<span>Schema and counts load on demand</span>
						<span className="font-medium text-foreground">Open table →</span>
					</div>
				</div>
			</button>
		</Card>
	);
};

interface RemoteSourceCardProps {
	name: string;
	sourceName: string;
	objectCount: number;
	onOpen: () => void;
}

const RemoteSourceCard: React.FC<RemoteSourceCardProps> = ({
	name,
	sourceName,
	objectCount,
	onOpen,
}) => {
	return (
		<Card className="group cursor-pointer overflow-hidden border transition-all duration-200 hover:bg-accent/50 hover:shadow-lg">
			<button
				type="button"
				onClick={onOpen}
				className="h-full w-full p-0 text-left"
				title={`Preview remote objects: ${name}`}
			>
				<div className="space-y-5 p-5">
					<div className="flex items-start justify-between gap-3">
						<div className="flex min-w-0 items-center gap-3">
							<div className="shrink-0 rounded-xl bg-sky-500/10 p-2.5 transition-colors group-hover:bg-sky-500/20">
								<Cloud className="h-5 w-5 text-sky-500" />
							</div>
							<div className="min-w-0">
								<h3 className="truncate text-sm font-semibold leading-tight">
									{name}
								</h3>
								<p className="truncate text-xs text-muted-foreground">
									from {sourceName}
								</p>
							</div>
						</div>
						<Badge
							variant="outline"
							className="shrink-0 gap-1 border-sky-500/20 bg-sky-500/10 text-[10px] text-sky-500"
						>
							<Cloud className="h-3 w-3" />
							Remote
						</Badge>
					</div>

					<div className="flex items-center justify-between border-t pt-3 text-xs text-muted-foreground">
						<span>
							{objectCount} object{objectCount === 1 ? "" : "s"}
						</span>
						<span className="font-medium text-foreground">
							Preview objects →
						</span>
					</div>
				</div>
			</button>
		</Card>
	);
};

const LOADING_CARD_KEYS = ["one", "two", "three", "four", "five", "six"];
const LOADING_COLUMN_KEYS = ["one", "two", "three", "four", "five"];
const LOADING_ROW_KEYS = [
	"one",
	"two",
	"three",
	"four",
	"five",
	"six",
	"seven",
	"eight",
];

const LoadingState: React.FC = () => (
	<div className="p-6">
		<div className="flex items-center gap-4 mb-6">
			<Database className="h-8 w-8 text-muted-foreground animate-pulse" />
			<div>
				<div className="h-8 w-48 bg-muted animate-pulse rounded mb-2" />
				<div className="h-4 w-72 bg-muted animate-pulse rounded" />
			</div>
		</div>
		<div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-4">
			{LOADING_CARD_KEYS.map((key) => (
				<Card key={key} className="animate-pulse bg-muted/50 p-5 space-y-4">
					<div className="flex items-center gap-3">
						<div className="h-10 w-10 rounded-xl bg-muted" />
						<div className="h-4 w-32 bg-muted rounded" />
					</div>
					<div className="grid grid-cols-3 gap-2">
						<div className="h-12 bg-muted rounded-lg" />
						<div className="h-12 bg-muted rounded-lg" />
						<div className="h-12 bg-muted rounded-lg" />
					</div>
					<div className="flex gap-1.5">
						<div className="h-5 w-14 bg-muted rounded" />
						<div className="h-5 w-12 bg-muted rounded" />
					</div>
				</Card>
			))}
		</div>
	</div>
);

const TableViewLoadingState: React.FC<{
	onBack: () => void;
	tableName: string;
}> = ({ onBack, tableName }) => (
	<div className="flex flex-col h-full grow max-h-full min-w-0 p-4 gap-4">
		<div className="flex items-center gap-3">
			<Button variant="default" size="sm" onClick={onBack}>
				<ArrowLeftIcon />
				Back
			</Button>
			<div className="flex items-center gap-2 min-w-0">
				<Database className="h-5 w-5 text-muted-foreground animate-pulse shrink-0" />
				<span className="text-sm font-medium truncate">{tableName}</span>
			</div>
		</div>
		<div className="flex items-center gap-2">
			<div className="h-9 w-24 bg-muted animate-pulse rounded" />
			<div className="h-9 flex-1 bg-muted animate-pulse rounded" />
			<div className="h-9 w-20 bg-muted animate-pulse rounded" />
		</div>
		<div className="flex-1 rounded border overflow-hidden">
			<div className="h-10 bg-muted/60 border-b flex items-center gap-4 px-4">
				{LOADING_COLUMN_KEYS.map((key, index) => (
					<div
						key={key}
						className="h-4 bg-muted animate-pulse rounded"
						style={{ width: `${60 + index * 20}px` }}
					/>
				))}
			</div>
			{LOADING_ROW_KEYS.map((rowKey, rowIndex) => (
				<div
					key={rowKey}
					className="h-10 border-b flex items-center gap-4 px-4"
				>
					{LOADING_COLUMN_KEYS.map((columnKey, columnIndex) => (
						<div
							key={`${rowKey}-${columnKey}`}
							className="h-3.5 bg-muted/50 animate-pulse rounded"
							style={{
								width: `${40 + ((rowIndex + columnIndex) % 4) * 25}px`,
							}}
						/>
					))}
				</div>
			))}
		</div>
		<div className="flex items-center justify-between shrink-0">
			<div className="h-4 w-32 bg-muted animate-pulse rounded" />
			<div className="flex gap-1">
				<div className="h-8 w-8 bg-muted animate-pulse rounded" />
				<div className="h-8 w-8 bg-muted animate-pulse rounded" />
			</div>
		</div>
	</div>
);

const OverlayView: React.FC<{
	appId: string;
	overlayId: string;
	onBack: () => void;
}> = ({ appId, overlayId, onBack }) => {
	const [overlay, setOverlay] = useState<GraphOverlay | null>(null);

	return (
		<div className="flex flex-col h-full min-h-0">
			<div className="flex items-center gap-3 p-4 border-b">
				<Button variant="ghost" size="icon" onClick={onBack}>
					<ArrowLeftIcon className="h-4 w-4" />
				</Button>
				<div>
					<h2 className="text-lg font-semibold">
						{overlay?.name ?? "Graph Overlay"}
					</h2>
					{overlay?.description && (
						<p className="text-xs text-muted-foreground">
							{overlay.description}
						</p>
					)}
				</div>
			</div>
			<div className="flex-1 min-h-0">
				<OntologyExplorer
					appId={appId}
					overlayId={overlayId}
					allowCypher
					allowStyleEdit
					onOverlayLoaded={setOverlay}
					renderError={(message) => (
						<div className="flex h-full items-center justify-center">
							<div className="text-center space-y-2">
								<p className="text-sm text-destructive">{message}</p>
								<Button variant="outline" onClick={onBack}>
									Go back
								</Button>
							</div>
						</div>
					)}
				/>
			</div>
		</div>
	);
};

const PartialFailureAlert: React.FC<{
	failures: { name: string; onRetry: () => void }[];
}> = ({ failures }) => {
	const names = failures.map((failure) => failure.name).join(", ");
	const retryAll = useCallback(() => {
		for (const failure of failures) failure.onRetry();
	}, [failures]);
	return (
		<div
			role="alert"
			className="mt-4 flex items-center justify-between gap-3 rounded-lg border border-amber-500/40 bg-amber-500/10 px-4 py-2.5 text-sm"
		>
			<div className="flex items-center gap-2 text-amber-600 dark:text-amber-400">
				<AlertTriangle className="h-4 w-4 shrink-0" />
				<span>Failed to load {names}. Some data may be missing.</span>
			</div>
			<Button
				variant="outline"
				size="sm"
				onClick={retryAll}
				className="shrink-0"
			>
				<RefreshCw className="mr-1.5 h-3.5 w-3.5" />
				Retry
			</Button>
		</div>
	);
};

const ErrorState: React.FC<{ onRetry: () => void }> = ({ onRetry }) => (
	<div className="p-6">
		<div className="rounded-lg border bg-card p-8 text-center">
			<Database className="mx-auto h-10 w-10 text-destructive mb-4" />
			<h3 className="text-lg font-semibold mb-2">Failed to load tables</h3>
			<p className="text-sm text-muted-foreground mb-4">
				There was an error loading the database tables.
			</p>
			<Button onClick={onRetry}>
				<RefreshCw className="mr-2 h-4 w-4" />
				Try again
			</Button>
		</div>
	</div>
);

const EmptyState: React.FC<{ onRetry: () => void }> = ({ onRetry }) => (
	<div className="p-6">
		<div className="rounded-lg border bg-card p-8 text-center">
			<Database className="mx-auto h-10 w-10 text-muted-foreground mb-4" />
			<h3 className="text-lg font-semibold mb-2">No tables found</h3>
			<p className="text-sm text-muted-foreground mb-4">
				This project doesn&apos;t appear to have any database tables yet.
			</p>
			<Button onClick={onRetry}>
				<RefreshCw className="mr-2 h-4 w-4" />
				Refresh
			</Button>
		</div>
	</div>
);
