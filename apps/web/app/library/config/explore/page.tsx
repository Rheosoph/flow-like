"use client";
import {
	Button,
	Card,
	CardHeader,
	CardTitle,
	IIndexType,
	Input,
	ScrollArea,
	useBackend,
	useInvoke,
} from "@tm9657/flow-like-ui";
import LanceDBExplorer from "@tm9657/flow-like-ui/components/ui/lance-viewer";
import {
	ArrowDownAZ,
	ArrowLeftIcon,
	ArrowUpAZ,
	ChevronRight,
	Columns,
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
		await backend.dbState.optimize(appId, table, userScoped);
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
				"full text": IIndexType.FullText,
				fulltext: IIndexType.FullText,
				btree: IIndexType.BTree,
				bitmap: IIndexType.Bitmap,
				"label list": IIndexType.LabelList,
				labellist: IIndexType.LabelList,
				auto: IIndexType.Auto,
			};
			const enumType = typeMap[indexType.toLowerCase()] ?? IIndexType.Auto;
			await backend.dbState.buildIndex(appId, table, column, enumType, userScoped);
			handleRefresh();
		},
		[backend.dbState, appId, table, userScoped, handleRefresh],
	);

	const isLoadingData = schema.isLoading || list.isLoading;

	if (isLoadingData && !schema.data) {
		return <TableViewLoadingState />;
	}

	return (
		<div className="flex flex-col h-full flex-grow max-h-full min-w-0">
			{schema.data && list.data && (
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
			)}
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

	if (tables.isLoading && userTables.isLoading) {
		return <LoadingState />;
	}

	if (tables.error && userTables.error) {
		return <ErrorState onRetry={refreshTables} />;
	}

	if (!processedTables.length) {
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
			<div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4 pr-2 py-1">
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

const TableCard: React.FC<TableCardProps> = ({ appId, table, onSelect }) => {
	const backend = useBackend();
	const count = useInvoke(backend.dbState.countItems, backend.dbState, [
		appId,
		table.name,
		table.userScoped,
	]);

	return (
		<Card className="group cursor-pointer transition-all duration-200 hover:shadow-lg hover:-translate-y-1 hover:bg-accent/50 border">
			<button
				onClick={onSelect}
				className="w-full h-full p-0 text-left"
				title={`Open table: ${table.name}`}
			>
				<CardHeader className="py-2 px-6">
					<div className="flex items-start justify-between gap-4 mb-4">
						<div className={`flex-shrink-0 rounded-xl p-3 transition-colors ${table.userScoped ? "bg-amber-500/10 group-hover:bg-amber-500/20" : "bg-primary/10 group-hover:bg-primary/20"}`}>
							{table.userScoped ? (
								<User className="h-5 w-5 text-amber-500" />
							) : (
								<Columns className="h-5 w-5 text-primary" />
							)}
						</div>
						<ChevronRight className="h-5 w-5 text-muted-foreground transition-all group-hover:translate-x-1 group-hover:text-primary flex-shrink-0 mt-0.5" />
					</div>

					<div className="space-y-2">
						<CardTitle className="text-base font-semibold leading-tight">
							{table.name}
						</CardTitle>
						<div className="flex items-center gap-2">
						{table.userScoped ? (
							<span className="inline-flex items-center gap-1 rounded-full bg-amber-500/10 px-2 py-0.5 text-xs font-medium text-amber-500">
								<User className="h-3 w-3" />
								User Scoped
							</span>
						) : (
							<span className="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary">
								<Globe className="h-3 w-3" />
								Shared
								</span>
							)}
							<p className="text-sm text-muted-foreground">
								{count.data !== undefined
									? `${count.data.toLocaleString()} items`
									: "Loading..."}
							</p>
						</div>
					</div>
				</CardHeader>
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
		<div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
			{Array.from({ length: 8 }).map((_, i) => (
				<Card key={i} className="h-20 animate-pulse bg-muted/50" />
			))}
		</div>
	</div>
);

const TableViewLoadingState: React.FC = () => (
	<div className="flex flex-col h-full flex-grow max-h-full min-w-0 p-4 gap-4">
		<div className="flex items-center gap-4">
			<div className="h-8 w-8 bg-muted animate-pulse rounded" />
			<div className="flex-1">
				<div className="h-6 w-48 bg-muted animate-pulse rounded mb-2" />
				<div className="h-4 w-32 bg-muted animate-pulse rounded" />
			</div>
		</div>
		<div className="flex items-center gap-2">
			<div className="h-9 w-24 bg-muted animate-pulse rounded" />
			<div className="h-9 flex-1 bg-muted animate-pulse rounded" />
		</div>
		<div className="flex-1 bg-muted/30 animate-pulse rounded border" />
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
