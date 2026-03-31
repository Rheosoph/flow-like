"use client";
import {
	Badge,
	Button,
	Card,
	IIndexType,
	Input,
	ScrollArea,
	cn,
	useBackend,
	useInvoke,
} from "@tm9657/flow-like-ui";
import LanceDBExplorer, {
	type ArrowSchemaJSON,
	arrowToLanceSchema,
} from "@tm9657/flow-like-ui/components/ui/lance-viewer";
import {
	ArrowDownAZ,
	ArrowLeftIcon,
	ArrowUpAZ,
	Database,
	Globe,
	RefreshCw,
	Search,
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
import { useCallback, useMemo, useState } from "react";
import NotFound from "../not-found";

export default function Page(): React.ReactElement {
	const router = useRouter();
	const searchParams = useSearchParams();
	const id = searchParams?.get("id") ?? null;
	const tableParam = searchParams?.get("table") ?? null;

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

	if (!id) return <NotFound />;

	return table ? (
		<TableView
			table={table}
			appId={id}
			userScoped={userScoped}
			onBack={() => {
				const params = new URLSearchParams(searchParams?.toString() ?? "");
				params.delete("table");
				params.delete("scope");
				router.push(`${pathname}?${params.toString()}`);
			}}
		/>
	) : (
		<DatabaseOverview appId={id} searchParams={searchParams} />
	);
}

function TableView({
	table,
	appId,
	userScoped,
	onBack,
}: Readonly<{ table: string; appId: string; userScoped?: boolean; onBack: () => void }>) {
	const backend = useBackend();
	const router = useRouter();
	const pathname = usePathname();
	const searchParams = useSearchParams();

	// Get page and pageSize from URL params
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
			await backend.dbState.updateItem(appId, table, filter, updates, userScoped);
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
			await backend.dbState.addColumn(appId, table, {
				name,
				sql_expression: sqlExpression,
			}, userScoped);
			handleRefresh();
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	const handleAlterColumn = useCallback(
		async (column: string, nullable: boolean) => {
			await backend.dbState.alterColumn(appId, table, column, nullable, userScoped);
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
			await backend.dbState.buildIndex(appId, table, column, enumType, undefined, userScoped);
			handleRefresh();
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	if (!schema.data || !list.data) {
		return <TableViewLoadingState onBack={onBack} tableName={table} />;
	}

	return (
		<div className="flex flex-col h-full flex-grow max-h-full min-w-0">
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
				<Button
					variant={"default"}
					size={"sm"}
					onClick={() => {
						onBack();
					}}
				>
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
	const userTables = useInvoke(backend.dbState.listTablesUser, backend.dbState, [
		appId,
	]);

	const [query, setQuery] = useState<string>("");
	const [sortAsc, setSortAsc] = useState<boolean>(true);

	const processedTables = useMemo(() => {
		const projectTables = (tables.data ?? []).map((name): Table => ({ name }));
		const userScopedTables = (userTables.data ?? []).map((name): Table => ({ name, userScoped: true }));
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
		<div className="p-6 space-y-6">
			<DatabaseHeader
				sortAsc={sortAsc}
				onToggleSort={toggleSort}
				onRefresh={refreshTables}
			/>

			<SearchInput value={query} onChange={setQuery} onClear={clearSearch} />

			<TableGrid
				appId={appId}
				tables={filteredAndSortedTables}
				onSelectTable={navigateToTable}
				searchQuery={query}
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
	<header className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between w-full flex-grow">
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
			onChange={(e) => onChange(e.target.value)}
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
		<ScrollArea className="max-h-[calc(100vh-16rem)]">
			<div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-4 pr-2 py-1">
				{tables.map((table) => (
					<TableCard
						appId={appId}
						key={`${table.userScoped ? "user:" : ""}${table.name}`}
						table={table}
						onSelect={() => onSelectTable(table.name, table.userScoped)}
					/>
				))}
			</div>
		</ScrollArea>
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

const vectorDtLabel = (raw: ArrowSchemaJSON | undefined, fieldName: string): string | null => {
	if (!raw) return null;
	const f = raw.fields?.find((f: any) => f.name === fieldName);
	if (!f?.data_type?.FixedSizeList) return null;
	const [child] = f.data_type.FixedSizeList;
	const dt = child?.data_type;
	if (dt === "Float16") return "f16";
	if (dt === "Float32") return "f32";
	if (dt === "Float64") return "f64";
	return null;
};

const StatBlock: React.FC<{ label: string; value: string }> = ({ label, value }) => (
	<div className="rounded-lg bg-muted/50 px-3 py-2">
		<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">{label}</p>
		<p className="text-lg font-semibold leading-tight mt-0.5">{value}</p>
	</div>
);

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
		() => parsedSchema?.fields.find((f) => f.kind === "vector"),
		[parsedSchema],
	);

	const hasVectors = !!vectorField;
	const columnCount = parsedSchema?.fields.length;
	const dimensions = vectorField?.dims;
	const dtLabel = vectorField ? vectorDtLabel(rawSchema.data, vectorField.name) : null;

	const indexTags = useMemo(() => {
		if (!indices.data?.length) return [];
		return indices.data.map((idx) => idx.index_type);
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
							<div className={cn(
								"shrink-0 rounded-xl p-2.5 transition-colors",
								table.userScoped
									? "bg-amber-500/10 group-hover:bg-amber-500/20"
									: "bg-primary/10 group-hover:bg-primary/20",
							)}>
								<Database className={cn("h-5 w-5", table.userScoped ? "text-amber-500" : "text-primary")} />
							</div>
							<div className="min-w-0">
								<h3 className="font-semibold text-sm leading-tight truncate">{table.name}</h3>
							</div>
						</div>
						{table.userScoped ? (
							<Badge variant="outline" className="shrink-0 bg-amber-500/10 text-amber-500 border-amber-500/20 text-[10px] gap-1">
								<User className="h-3 w-3" />
								User scoped
							</Badge>
						) : (
							<Badge variant="outline" className="shrink-0 bg-primary/10 text-primary border-primary/20 text-[10px] gap-1">
								<Globe className="h-3 w-3" />
								Shared
							</Badge>
						)}
					</div>

					<div className={cn("grid gap-2", hasVectors ? "grid-cols-3" : "grid-cols-2")}>
						<StatBlock label="Rows" value={formatCount(count.data)} />
						{hasVectors && <StatBlock label="Dimensions" value={formatCount(dimensions)} />}
						<StatBlock label="Columns" value={formatCount(columnCount)} />
					</div>

					<div className="flex flex-wrap gap-1.5">
						{hasVectors ? (
							<>
								{indexTags.length > 0
									? indexTags.map((tag) => (
											<Badge key={tag} variant="secondary" className="text-[10px] font-medium">
												{tag}
											</Badge>
										))
									: (
										<Badge variant="secondary" className="text-[10px] font-medium">
											no index
										</Badge>
									)}
								{dtLabel && (
									<Badge variant="secondary" className="text-[10px] font-medium">
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
											variant={field.kind === "vector" ? "default" : "secondary"}
											className="text-[10px] font-medium"
										>
											{field.name}
										</Badge>
									))}
									{parsedSchema.fields.length > 6 && (
										<Badge variant="secondary" className="text-[10px] font-medium">
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
			{Array.from({ length: 6 }).map((_, i) => (
				<Card key={i} className="animate-pulse bg-muted/50 p-5 space-y-4">
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

const TableViewLoadingState: React.FC<{ onBack: () => void; tableName: string }> = ({ onBack, tableName }) => (
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
				{Array.from({ length: 5 }).map((_, i) => (
					<div key={i} className="h-4 bg-muted animate-pulse rounded" style={{ width: `${60 + i * 20}px` }} />
				))}
			</div>
			{Array.from({ length: 8 }).map((_, i) => (
				<div key={i} className="h-10 border-b flex items-center gap-4 px-4">
					{Array.from({ length: 5 }).map((_, j) => (
						<div key={j} className="h-3.5 bg-muted/50 animate-pulse rounded" style={{ width: `${40 + ((i + j) % 4) * 25}px` }} />
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
