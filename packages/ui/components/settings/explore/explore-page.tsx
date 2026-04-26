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
	useBackend,
	useInvoke,
} from "@tm9657/flow-like-ui";
import {
	GraphViewer,
	getGraphIcon,
} from "@tm9657/flow-like-ui/components/ui/graph";
import {
	OverlayWizard,
	type TableInfo,
} from "@tm9657/flow-like-ui/components/ui/graph/overlay-builder";
import LanceDBExplorer, {
	type ArrowSchemaJSON,
	arrowToLanceSchema,
} from "@tm9657/flow-like-ui/components/ui/lance-viewer";
import type {
	CreateOverlayPayload,
	GraphOverlay,
	LabelStyle,
	PropertyColumn,
	SubgraphNode,
	SubgraphResult,
	ValidationResult,
} from "@tm9657/flow-like-ui/state/backend-state/graph-state";
import {
	ArrowDownAZ,
	ArrowLeftIcon,
	ArrowUpAZ,
	Database,
	Eye,
	Globe,
	Network,
	Plus,
	RefreshCw,
	Search,
	Trash2,
	User,
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
		? Number.parseInt(pageSizeParam, 10) || 25
		: 25;
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

	const updateUrlParams = useCallback(
		(newPage: number, newPageSize: number) => {
			const params = new URLSearchParams(searchParams?.toString() ?? "");
			if (newPage > 1) {
				params.set("page", String(newPage));
			} else {
				params.delete("page");
			}
			if (newPageSize !== 25) {
				params.set("pageSize", String(newPageSize));
			} else {
				params.delete("pageSize");
			}
			router.replace(`${pathname}?${params.toString()}`, { scroll: false });
		},
		[router, pathname, searchParams],
	);

	const handleRefresh = useCallback(() => {
		schema.refetch();
		count.refetch();
		list.refetch();
	}, [schema, count, list]);

	const handleOptimize = useCallback(async () => {
		await backend.dbState.optimize(appId, table, undefined, userScoped);
		handleRefresh();
	}, [backend.dbState, appId, table, userScoped, handleRefresh]);

	const handleUpdateItem = useCallback(
		async (filter: string, updates: Record<string, unknown>) => {
			await backend.dbState.updateItem(
				appId,
				table,
				filter,
				updates,
				userScoped,
			);
			handleRefresh();
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	const handleDropColumns = useCallback(
		async (columns: string[]) => {
			await backend.dbState.dropColumns(appId, table, columns, userScoped);
			handleRefresh();
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	const handleAddColumn = useCallback(
		async (name: string, sqlExpression: string) => {
			await backend.dbState.addColumn(
				appId,
				table,
				{
					name,
					sql_expression: sqlExpression,
				},
				userScoped,
			);
			handleRefresh();
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	const handleAlterColumn = useCallback(
		async (column: string, nullable: boolean) => {
			await backend.dbState.alterColumn(
				appId,
				table,
				column,
				nullable,
				userScoped,
			);
			handleRefresh();
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	const handleGetIndices = useCallback(async () => {
		return backend.dbState.getIndices(appId, table, userScoped);
	}, [backend.dbState, appId, table, userScoped]);

	const handleDropIndex = useCallback(
		async (indexName: string) => {
			await backend.dbState.dropIndex(appId, table, indexName, userScoped);
			handleRefresh();
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
			await backend.dbState.buildIndex(
				appId,
				table,
				column,
				enumType,
				undefined,
				userScoped,
			);
			handleRefresh();
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
	const router = useRouter();
	const pathname = usePathname();
	const tables = useInvoke(backend.dbState.listTables, backend.dbState, [
		appId,
	]);
	const userTables = useInvoke(
		backend.dbState.listTablesUser,
		backend.dbState,
		[appId],
	);

	const [query, setQuery] = useState<string>("");
	const [sortAsc, setSortAsc] = useState<boolean>(true);

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
			params.set("table", encodeURIComponent(tableName));
			if (userScoped) {
				params.set("scope", "user");
			}
			router.push(`${pathname}?${params.toString()}`);
		},
		[router, pathname, searchParams],
	);

	const refreshTables = useCallback(() => {
		tables.refetch();
		userTables.refetch();
	}, [tables.refetch, userTables.refetch]);

	const clearSearch = useCallback(() => {
		setQuery("");
	}, []);

	const toggleSort = useCallback(() => {
		setSortAsc((prev) => !prev);
	}, []);

	const isLoading = tables.isLoading || userTables.isLoading;

	if (isLoading && !processedTables.length) {
		return <LoadingState />;
	}

	if (!isLoading && tables.error && userTables.error) {
		return <ErrorState onRetry={refreshTables} />;
	}

	if (!isLoading && !processedTables.length) {
		return <EmptyState onRetry={refreshTables} />;
	}

	return (
		<div className="flex flex-col h-full">
			<div className="p-6 pb-0">
				<DatabaseHeader
					sortAsc={sortAsc}
					onToggleSort={toggleSort}
					onRefresh={refreshTables}
				/>
			</div>
			<Tabs defaultValue="tables" className="flex flex-col flex-1 min-h-0">
				<div className="px-6 pt-4">
					<TabsList>
						<TabsTrigger value="tables">
							<Database className="h-3.5 w-3.5 mr-1.5" />
							Tables
						</TabsTrigger>
						<TabsTrigger value="overlays">
							<Network className="h-3.5 w-3.5 mr-1.5" />
							Graph Overlays
						</TabsTrigger>
					</TabsList>
				</div>
				<TabsContent
					value="tables"
					className="flex-1 overflow-y-auto p-6 space-y-4"
				>
					<SearchInput
						value={query}
						onChange={setQuery}
						onClear={clearSearch}
					/>
					<TableGrid
						appId={appId}
						tables={filteredAndSortedTables}
						onSelectTable={navigateToTable}
						searchQuery={query}
					/>
				</TabsContent>
				<TabsContent value="overlays" className="flex-1 overflow-y-auto p-6">
					<GraphOverlaySection appId={appId} tables={processedTables} />
				</TabsContent>
			</Tabs>
		</div>
	);
};

function arrowDataTypeToString(dt: unknown): string {
	if (typeof dt === "string") return dt;
	if (dt && typeof dt === "object") {
		const key = Object.keys(dt)[0];
		if (key) return key;
	}
	return "Unknown";
}

function arrowSchemaToPropertyColumns(
	schema: ArrowSchemaJSON | undefined,
): PropertyColumn[] {
	if (!schema?.fields) return [];
	return schema.fields.map((field: any) => ({
		name: String(field?.name ?? ""),
		data_type: arrowDataTypeToString(field?.data_type),
		nullable: field?.nullable ?? true,
	}));
}

const GraphOverlaySection: React.FC<{ appId: string; tables: Table[] }> = ({
	appId,
	tables,
}) => {
	const backend = useBackend();
	const router = useRouter();
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const overlays = useInvoke(
		backend.graphState.listOverlays,
		backend.graphState,
		[appId],
	);

	const [wizardOpen, setWizardOpen] = useState(false);
	const [submitting, setSubmitting] = useState(false);
	const [validation, setValidation] = useState<ValidationResult | null>(null);
	const [tableColumns, setTableColumns] = useState<
		Record<string, PropertyColumn[]>
	>({});

	useEffect(() => {
		if (!wizardOpen) return;
		let cancelled = false;
		const fetchSchemas = async () => {
			const result: Record<string, PropertyColumn[]> = {};
			await Promise.all(
				tables.map(async (table) => {
					try {
						const schema = await backend.dbState.getSchema(
							appId,
							table.name,
							table.userScoped,
						);
						if (!cancelled) {
							result[table.name] = arrowSchemaToPropertyColumns(schema);
						}
					} catch {
						return;
					}
				}),
			);
			if (!cancelled) setTableColumns(result);
		};
		fetchSchemas();
		return () => {
			cancelled = true;
		};
	}, [wizardOpen, tables, backend.dbState, appId]);

	const wizardTables = useMemo<TableInfo[]>(
		() =>
			tables.map((table) => ({
				name: table.name,
				userScoped: table.userScoped,
			})),
		[tables],
	);

	const handleDelete = useCallback(
		async (event: React.MouseEvent, overlayId: string) => {
			event.stopPropagation();
			await backend.graphState.deleteOverlay(appId, overlayId);
			overlays.refetch();
		},
		[backend.graphState, appId, overlays],
	);

	const navigateToOverlay = useCallback(
		(overlayId: string) => {
			const params = new URLSearchParams(searchParams?.toString() ?? "");
			params.set("overlay", overlayId);
			router.push(`${pathname}?${params.toString()}`);
		},
		[router, pathname, searchParams],
	);

	const handleCreate = useCallback(
		async (payload: CreateOverlayPayload) => {
			setSubmitting(true);
			try {
				await backend.graphState.createOverlay(appId, payload);
				setWizardOpen(false);
				overlays.refetch();
			} finally {
				setSubmitting(false);
			}
		},
		[backend.graphState, appId, overlays],
	);

	const handleValidate = useCallback(async () => {
		setValidation(null);
	}, []);

	if (overlays.isLoading && !overlays.data) return null;

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-end">
				<Button size="sm" variant="outline" onClick={() => setWizardOpen(true)}>
					<Plus className="h-3.5 w-3.5 mr-1" />
					Create Overlay
				</Button>
			</div>

			{(overlays.data ?? []).length > 0 ? (
				<div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-4">
					{(overlays.data ?? []).map((overlay: GraphOverlay) => (
						<Card
							key={overlay.id}
							className="p-5 space-y-3 cursor-pointer transition-all duration-200 hover:shadow-lg hover:bg-accent/50"
							onClick={() => navigateToOverlay(overlay.id)}
						>
							<div className="flex items-start justify-between gap-2">
								<div className="min-w-0">
									<h3 className="text-sm font-semibold truncate">
										{overlay.name}
									</h3>
									{overlay.description && (
										<p className="text-xs text-muted-foreground truncate">
											{overlay.description}
										</p>
									)}
								</div>
								<div className="flex items-center gap-1 shrink-0">
									<Button
										variant="ghost"
										size="icon"
										className="h-7 w-7 text-muted-foreground hover:text-foreground"
										onClick={(event) => {
											event.stopPropagation();
											navigateToOverlay(overlay.id);
										}}
									>
										<Eye className="h-3.5 w-3.5" />
									</Button>
									<Button
										variant="ghost"
										size="icon"
										className="h-7 w-7 text-destructive"
										onClick={(event) => handleDelete(event, overlay.id)}
									>
										<Trash2 className="h-3.5 w-3.5" />
									</Button>
								</div>
							</div>
							<div className="flex gap-2">
								<Badge variant="secondary" className="text-[10px]">
									{overlay.nodes.length} node
									{overlay.nodes.length !== 1 ? "s" : ""}
								</Badge>
								<Badge variant="secondary" className="text-[10px]">
									{overlay.edges.length} edge
									{overlay.edges.length !== 1 ? "s" : ""}
								</Badge>
							</div>
							<div className="space-y-2">
								{overlay.nodes.length > 0 && (
									<div className="space-y-1">
										<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
											Nodes
										</p>
										<div className="flex flex-wrap gap-1.5">
											{overlay.nodes.slice(0, 4).map((node) => (
												<OverlayStyleChip
													key={`node-${node.label}`}
													label={node.label}
													style={node.style}
													type="node"
												/>
											))}
											{overlay.nodes.length > 4 && (
												<Badge variant="secondary" className="text-[10px]">
													+{overlay.nodes.length - 4}
												</Badge>
											)}
										</div>
									</div>
								)}
								{overlay.edges.length > 0 && (
									<div className="space-y-1">
										<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
											Edges
										</p>
										<div className="flex flex-wrap gap-1.5">
											{overlay.edges.slice(0, 4).map((edge) => (
												<OverlayStyleChip
													key={`edge-${edge.label}`}
													label={edge.label}
													style={edge.style}
													type="edge"
												/>
											))}
											{overlay.edges.length > 4 && (
												<Badge variant="secondary" className="text-[10px]">
													+{overlay.edges.length - 4}
												</Badge>
											)}
										</div>
									</div>
								)}
							</div>
						</Card>
					))}
				</div>
			) : (
				<Card className="p-8 text-center">
					<Network className="mx-auto h-8 w-8 text-muted-foreground mb-3" />
					<p className="text-sm text-muted-foreground mb-3">
						No graph overlays yet. Create one to visualize relationships between
						your tables.
					</p>
					<Button
						size="sm"
						variant="outline"
						onClick={() => setWizardOpen(true)}
					>
						<Plus className="h-3.5 w-3.5 mr-1" />
						Create Overlay
					</Button>
				</Card>
			)}

			<OverlayWizard
				open={wizardOpen}
				onClose={() => setWizardOpen(false)}
				onSubmit={handleCreate}
				tables={wizardTables}
				tableColumns={tableColumns}
				validation={validation}
				onValidate={handleValidate}
				submitting={submitting}
			/>
		</div>
	);
};

interface DatabaseHeaderProps {
	sortAsc: boolean;
	onToggleSort: () => void;
	onRefresh: () => void;
}

const DatabaseHeader: React.FC<DatabaseHeaderProps> = ({
	sortAsc,
	onToggleSort,
	onRefresh,
}) => (
	<header className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between w-full grow">
		<div className="flex items-center gap-4 w-full">
			<Database className="h-8 w-8 text-primary" />
			<div>
				<h1 className="text-2xl font-semibold">Database Tables</h1>
				<p className="text-sm text-muted-foreground">
					Browse and inspect your project&apos;s database schema
				</p>
			</div>
		</div>

		<div className="flex flex-row items-center gap-2 justify-end w-full">
			<Button
				variant="ghost"
				size="icon"
				onClick={onToggleSort}
				title={`Sort ${sortAsc ? "descending" : "ascending"}`}
			>
				{sortAsc ? (
					<ArrowUpAZ className="h-4 w-4" />
				) : (
					<ArrowDownAZ className="h-4 w-4" />
				)}
			</Button>
			<Button variant="outline" size="sm" onClick={onRefresh}>
				<RefreshCw className="mr-2 h-4 w-4" />
				Refresh
			</Button>
		</div>
	</header>
);

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
	appId: string;
	tables: Table[];
	onSelectTable: (tableName: string, userScoped?: boolean) => void;
	searchQuery: string;
}

const TableGrid: React.FC<TableGridProps> = ({
	appId,
	tables,
	onSelectTable,
	searchQuery,
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

	return (
		<div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-4">
			{tables.map((table) => (
				<TableCard
					appId={appId}
					key={`${table.userScoped ? "user:" : ""}${table.name}`}
					table={table}
					onSelect={() => onSelectTable(table.name, table.userScoped)}
				/>
			))}
		</div>
	);
};

interface TableCardProps {
	appId: string;
	table: Table;
	onSelect: () => void;
}

const formatCount = (n: number | undefined): string => {
	if (n === undefined) return "—";
	if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
	if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
	return n.toLocaleString();
};

const vectorDtLabel = (
	raw: ArrowSchemaJSON | undefined,
	fieldName: string,
): string | null => {
	if (!raw) return null;
	const field = raw.fields?.find((item: any) => item.name === fieldName);
	if (!field?.data_type?.FixedSizeList) return null;
	const [child] = field.data_type.FixedSizeList;
	const dataType = child?.data_type;
	if (dataType === "Float16") return "f16";
	if (dataType === "Float32") return "f32";
	if (dataType === "Float64") return "f64";
	return null;
};

const StatBlock: React.FC<{ label: string; value: string }> = ({
	label,
	value,
}) => (
	<div className="rounded-lg bg-muted/50 px-3 py-2">
		<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
			{label}
		</p>
		<p className="text-lg font-semibold leading-tight mt-0.5">{value}</p>
	</div>
);

const OverlayStyleChip: React.FC<{
	label: string;
	style: LabelStyle;
	type: "node" | "edge";
}> = ({ label, style, type }) => {
	const Icon = getGraphIcon(style.icon);

	return (
		<div className="flex max-w-full items-center gap-1.5 rounded-full border bg-background/60 px-1.5 py-0.5 text-[10px]">
			<span
				className="flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-white"
				style={{ backgroundColor: style.color }}
			>
				<Icon className="h-2.5 w-2.5" />
			</span>
			{type === "edge" && (
				<span
					className="h-0.5 w-3 shrink-0 rounded-full"
					style={{ backgroundColor: style.color }}
				/>
			)}
			<span className="truncate">{label}</span>
		</div>
	);
};

const TableCard: React.FC<TableCardProps> = ({ appId, table, onSelect }) => {
	const backend = useBackend();
	const count = useInvoke(backend.dbState.countItems, backend.dbState, [
		appId,
		table.name,
		table.userScoped,
	]);
	const rawSchema = useInvoke(backend.dbState.getSchema, backend.dbState, [
		appId,
		table.name,
		table.userScoped,
	]);
	const indices = useInvoke(backend.dbState.getIndices, backend.dbState, [
		appId,
		table.name,
		table.userScoped,
	]);

	const parsedSchema = useMemo(() => {
		if (!rawSchema.data) return null;
		return arrowToLanceSchema(rawSchema.data);
	}, [rawSchema.data]);

	const vectorField = useMemo(
		() => parsedSchema?.fields.find((field) => field.kind === "vector"),
		[parsedSchema],
	);

	const hasVectors = !!vectorField;
	const columnCount = parsedSchema?.fields.length;
	const dimensions = vectorField?.dims;
	const dtLabel = vectorField
		? vectorDtLabel(rawSchema.data, vectorField.name)
		: null;

	const indexTags = useMemo(() => {
		if (!indices.data?.length) return [];
		return indices.data.map((index) => index.index_type);
	}, [indices.data]);

	return (
		<Card className="group cursor-pointer transition-all duration-200 hover:shadow-lg hover:bg-accent/50 border overflow-hidden">
			<button
				onClick={onSelect}
				className="w-full h-full p-0 text-left"
				title={`Open table: ${table.name}`}
			>
				<div className="p-5 space-y-4">
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

					<div
						className={cn(
							"grid gap-2",
							hasVectors ? "grid-cols-3" : "grid-cols-2",
						)}
					>
						<StatBlock label="Rows" value={formatCount(count.data)} />
						{hasVectors && (
							<StatBlock label="Dimensions" value={formatCount(dimensions)} />
						)}
						<StatBlock label="Columns" value={formatCount(columnCount)} />
					</div>

					<div className="flex flex-wrap gap-1.5">
						{hasVectors ? (
							<>
								{indexTags.length > 0 ? (
									indexTags.map((tag) => (
										<Badge
											key={tag}
											variant="secondary"
											className="text-[10px] font-medium"
										>
											{tag}
										</Badge>
									))
								) : (
									<Badge
										variant="secondary"
										className="text-[10px] font-medium"
									>
										no index
									</Badge>
								)}
								{dtLabel && (
									<Badge
										variant="secondary"
										className="text-[10px] font-medium"
									>
										{dtLabel}
									</Badge>
								)}
							</>
						) : (
							<span className="text-xs text-muted-foreground italic">
								Tabular — no vectors
							</span>
						)}
					</div>

					{parsedSchema && parsedSchema.fields.length > 0 && (
						<>
							<div className="border-t" />
							<div className="space-y-2">
								<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
									Schema Preview
								</p>
								<div className="flex flex-wrap gap-1.5">
									{parsedSchema.fields.slice(0, 6).map((field) => (
										<Badge
											key={field.name}
											variant={
												field.kind === "vector" ? "default" : "secondary"
											}
											className="text-[10px] font-medium"
										>
											{field.name}
										</Badge>
									))}
									{parsedSchema.fields.length > 6 && (
										<Badge
											variant="secondary"
											className="text-[10px] font-medium"
										>
											+{parsedSchema.fields.length - 6}
										</Badge>
									)}
								</div>
							</div>
						</>
					)}
				</div>
			</button>
		</Card>
	);
};

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
			{Array.from({ length: 6 }).map((_, index) => (
				<Card key={index} className="animate-pulse bg-muted/50 p-5 space-y-4">
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
				{Array.from({ length: 5 }).map((_, index) => (
					<div
						key={index}
						className="h-4 bg-muted animate-pulse rounded"
						style={{ width: `${60 + index * 20}px` }}
					/>
				))}
			</div>
			{Array.from({ length: 8 }).map((_, rowIndex) => (
				<div
					key={rowIndex}
					className="h-10 border-b flex items-center gap-4 px-4"
				>
					{Array.from({ length: 5 }).map((_, columnIndex) => (
						<div
							key={columnIndex}
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

function enrichSubgraphWithStyles(
	result: SubgraphResult,
	overlay: GraphOverlay,
): SubgraphResult {
	const nodeStyleMap = new Map(
		overlay.nodes.map((node) => [node.label, node.style]),
	);
	const edgeStyleMap = new Map(
		overlay.edges.map((edge) => [edge.label, edge.style]),
	);

	const defaultStyle = {
		color: "#6b7280",
		icon: "circle",
		size: { mode: "fixed" as const, value: 6 },
	};

	return {
		...result,
		nodes: result.nodes.map((node) => ({
			...node,
			style: nodeStyleMap.get(node.label) ?? node.style ?? defaultStyle,
		})),
		edges: result.edges.map((edge) => ({
			...edge,
			style: edgeStyleMap.get(edge.label) ?? edge.style ?? defaultStyle,
		})),
	};
}

function mergeSubgraphData(
	current: SubgraphResult | null,
	incoming: SubgraphResult,
): SubgraphResult {
	if (!current) return incoming;

	const nodeIds = new Set(current.nodes.map((node) => node.id));
	const edgeIds = new Set(current.edges.map((edge) => edge.id));

	return {
		nodes: [
			...current.nodes,
			...incoming.nodes.filter((node) => !nodeIds.has(node.id)),
		],
		edges: [
			...current.edges,
			...incoming.edges.filter((edge) => !edgeIds.has(edge.id)),
		],
		truncated: current.truncated || incoming.truncated,
	};
}

const GRAPH_MAX_NODE_LIMIT = 1_000_000;
const GRAPH_NODE_EXPANSION_LIMIT = GRAPH_MAX_NODE_LIMIT;
const GRAPH_SEARCH_MATCH_LIMIT = 12;
const GRAPH_VIEW_LIMIT_MAX = GRAPH_MAX_NODE_LIMIT;

const OverlayView: React.FC<{
	appId: string;
	overlayId: string;
	onBack: () => void;
}> = ({ appId, overlayId, onBack }) => {
	const backend = useBackend();
	const [overlay, setOverlay] = useState<GraphOverlay | null>(null);
	const [data, setData] = useState<SubgraphResult | null>(null);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const [cypherResults, setCypherResults] = useState<unknown[] | null>(null);
	const [cypherLoading, setCypherLoading] = useState(false);
	const [cypherError, setCypherError] = useState<string | null>(null);
	const [nodeLimit, setNodeLimit] = useState(200);

	const loadInitialData = useCallback(
		async (currentOverlay: GraphOverlay, limitOverride?: number) => {
			setLoading(true);
			setError(null);
			try {
				const graphLimit = Math.min(
					limitOverride ?? currentOverlay.default_limit ?? 200,
					GRAPH_VIEW_LIMIT_MAX,
				);

				const result = await backend.graphState.subgraph(appId, overlayId, {
					seeds: [],
					depth: 1,
					limit: graphLimit,
				});
				setData(enrichSubgraphWithStyles(result, currentOverlay));
			} catch (err) {
				setError(extractErrorMessage(err));
				setData({ nodes: [], edges: [], truncated: false });
			} finally {
				setLoading(false);
			}
		},
		[backend.graphState, appId, overlayId],
	);

	useEffect(() => {
		let cancelled = false;
		(async () => {
			try {
				const currentOverlay = await backend.graphState.getOverlay(
					appId,
					overlayId,
				);
				if (cancelled) return;
				setOverlay(currentOverlay);
				const initialLimit = Math.min(
					currentOverlay.default_limit ?? 200,
					GRAPH_VIEW_LIMIT_MAX,
				);
				setNodeLimit(initialLimit);
				await loadInitialData(currentOverlay, initialLimit);
			} catch (err) {
				if (!cancelled) {
					setError(extractErrorMessage(err));
					setLoading(false);
				}
			}
		})();
		return () => {
			cancelled = true;
		};
	}, [backend.graphState, appId, overlayId, loadInitialData]);

	const handleRunCypher = useCallback(
		async (query: string) => {
			setCypherLoading(true);
			setCypherError(null);
			try {
				const results = await backend.graphState.cypher(appId, overlayId, {
					query,
				});
				setCypherResults(results);
			} catch (err) {
				setCypherError(extractErrorMessage(err));
			} finally {
				setCypherLoading(false);
			}
		},
		[backend.graphState, appId, overlayId],
	);

	const handleExpandNode = useCallback(
		async (
			nodeId: string,
			label: string,
			rawId?: unknown,
			seedNode?: SubgraphNode,
		) => {
			if (!overlay) return;

			if (seedNode) {
				setData((prev) =>
					mergeSubgraphData(
						prev,
						enrichSubgraphWithStyles(
							{ nodes: [seedNode], edges: [], truncated: false },
							overlay,
						),
					),
				);
			}

			setLoading(true);
			try {
				const prefix = `${label}:`;
				const resolvedId =
					rawId ??
					(nodeId.startsWith(prefix) ? nodeId.slice(prefix.length) : nodeId);
				const result = await backend.graphState.neighbors(appId, overlayId, {
					label,
					node_id: resolvedId,
					depth: 1,
					direction: "both",
					limit: GRAPH_NODE_EXPANSION_LIMIT,
				});
				const enriched = enrichSubgraphWithStyles(result, overlay);
				setData((prev) => mergeSubgraphData(prev, enriched));
			} catch (err) {
				toast.error(`Failed to expand neighbors: ${extractErrorMessage(err)}`);
			} finally {
				setLoading(false);
			}
		},
		[backend.graphState, appId, overlayId, overlay],
	);

	const handleSearchNodes = useCallback(
		async (query: string) =>
			backend.graphState.searchNodes(appId, overlayId, {
				query,
				limit: GRAPH_SEARCH_MATCH_LIMIT,
			}),
		[backend.graphState, appId, overlayId],
	);

	const handleLimitChange = useCallback(
		(newLimit: number) => {
			const clampedLimit = Math.min(newLimit, GRAPH_VIEW_LIMIT_MAX);
			setNodeLimit(clampedLimit);
			if (overlay) {
				loadInitialData(overlay, clampedLimit);
			}
		},
		[overlay, loadInitialData],
	);

	const handleStyleChange = useCallback(
		async (label: string, type: "node" | "edge", style: LabelStyle) => {
			if (!overlay) return;
			const updatedOverlay = { ...overlay };
			if (type === "node") {
				updatedOverlay.nodes = overlay.nodes.map((node) =>
					node.label === label ? { ...node, style } : node,
				);
			} else {
				updatedOverlay.edges = overlay.edges.map((edge) =>
					edge.label === label ? { ...edge, style } : edge,
				);
			}
			setOverlay(updatedOverlay);
			setData((prev) =>
				prev ? enrichSubgraphWithStyles(prev, updatedOverlay) : prev,
			);
			try {
				await backend.graphState.updateOverlay(appId, overlayId, {
					nodes: updatedOverlay.nodes,
					edges: updatedOverlay.edges,
				});
			} catch {
				return;
			}
		},
		[backend.graphState, appId, overlayId, overlay],
	);

	if (error) {
		return (
			<div className="flex flex-col h-full">
				<div className="flex items-center gap-3 p-4 border-b">
					<Button variant="ghost" size="icon" onClick={onBack}>
						<ArrowLeftIcon className="h-4 w-4" />
					</Button>
					<h2 className="text-lg font-semibold">Graph Overlay</h2>
				</div>
				<div className="flex-1 flex items-center justify-center">
					<div className="text-center space-y-2">
						<p className="text-sm text-destructive">{error}</p>
						<Button variant="outline" onClick={onBack}>
							Go back
						</Button>
					</div>
				</div>
			</div>
		);
	}

	if (!overlay) {
		return (
			<div className="flex flex-col h-full">
				<div className="flex items-center gap-3 p-4 border-b">
					<Button variant="ghost" size="icon" onClick={onBack}>
						<ArrowLeftIcon className="h-4 w-4" />
					</Button>
					<h2 className="text-lg font-semibold">Loading...</h2>
				</div>
				<div className="flex-1 flex items-center justify-center">
					<span className="text-sm text-muted-foreground animate-pulse">
						Loading overlay...
					</span>
				</div>
			</div>
		);
	}

	return (
		<div className="flex flex-col h-full min-h-0">
			<div className="flex items-center gap-3 p-4 border-b">
				<Button variant="ghost" size="icon" onClick={onBack}>
					<ArrowLeftIcon className="h-4 w-4" />
				</Button>
				<div>
					<h2 className="text-lg font-semibold">{overlay.name}</h2>
					{overlay.description && (
						<p className="text-xs text-muted-foreground">
							{overlay.description}
						</p>
					)}
				</div>
			</div>
			<div className="flex-1 min-h-0">
				<GraphViewer
					overlay={overlay}
					data={data}
					loading={loading}
					truncated={data?.truncated}
					onRunCypher={handleRunCypher}
					cypherResults={cypherResults}
					cypherLoading={cypherLoading}
					cypherError={cypherError}
					onExpandNode={handleExpandNode}
					onSearchNodes={handleSearchNodes}
					onStyleChange={handleStyleChange}
					onLimitChange={handleLimitChange}
					limit={nodeLimit}
				/>
			</div>
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
