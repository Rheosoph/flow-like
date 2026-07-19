// dnd-kit imports
import {
	DndContext,
	type DragEndEvent,
	PointerSensor,
	closestCenter,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import {
	SortableContext,
	arrayMove,
	horizontalListSortingStrategy,
	useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
	type ColumnDef,
	type ColumnFiltersState,
	type ColumnOrderState,
	type ColumnSizingState,
	type SortingState,
	type VisibilityState,
	flexRender,
	getCoreRowModel,
	getFilteredRowModel,
	getSortedRowModel,
	useReactTable,
} from "@tanstack/react-table";
import Dexie, { type Table } from "dexie";
import {
	ClipboardList,
	Clock,
	Columns3,
	Database,
	Download,
	GripVertical,
	Info,
	ListTree,
	Maximize2,
	Minimize2,
	MoreHorizontal,
	RefreshCcw,
	Save,
	Search,
	Settings,
	SlidersHorizontal,
	Trash2,
	Wrench,
	X,
	Zap,
} from "lucide-react";
import * as React from "react";
import { useCallback, useContext, useEffect, useMemo, useState } from "react";
import { cn } from "../../lib";
import type { IIndexConfig } from "../../state/backend-state/db-state";
import {
	Badge,
	Button,
	Checkbox,
	Dialog,
	DialogContent,
	DialogHeader,
	DialogTitle,
	DropdownMenu,
	DropdownMenuCheckboxItem,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
	Input,
	Label,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Switch,
	TextEditor,
	Textarea,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "./";
import {
	Table as DataTable,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "./table";
import {
	ColumnTypeSelect,
	EDIT_COLUMN_TYPE_GROUPS,
	IndexTypeSelect,
	buildAddColumnExpression,
} from "./table-schema";

export type LanceFieldKind =
	| "string"
	| "number"
	| "boolean"
	| "date"
	| "vector"
	| "array"
	| "object"
	| "unknown";

export interface LanceField {
	name: string;
	kind: LanceFieldKind;
	dims?: number;
	items?: LanceFieldKind | LanceField;
	nullable?: boolean;
}

export interface LanceSchema {
	table: string;
	fields: LanceField[];
	primaryKey?: string;
}

export interface ArrowSchemaJSON {
	fields: any[];
	metadata?: Record<string, unknown>;
}

export interface LanceDBExplorerProps {
	arrowSchema?: ArrowSchemaJSON;

	// Controlled data from parent (e.g., React Query)
	rows?: Record<string, any>[];
	total?: number;
	loading?: boolean;
	error?: string | null;

	// Notify parent when page/size/query change
	onPageRequest?: (args: {
		page: number;
		pageSize: number;
		offset: number;
		limit: number;
		query?: string;
	}) => void;

	// Callbacks for database operations
	onUpdateItem?: (
		filter: string,
		updates: Record<string, any>,
	) => Promise<void>;
	onOptimize?: (keepVersions?: boolean) => Promise<void>;
	onDropColumns?: (columns: string[]) => Promise<void>;
	onAddColumn?: (name: string, sqlExpression: string) => Promise<void>;
	onAlterColumn?: (column: string, nullable: boolean) => Promise<void>;
	onBuildIndex?: (column: string, indexType: string) => Promise<void>;
	onGetIndices?: () => Promise<IIndexConfig[]>;
	onDropIndex?: (indexName: string) => Promise<void>;
	onRefresh?: () => void;

	pageSizeOptions?: readonly number[];
	initialPage?: number;
	initialPageSize?: number;
	initialMode?: "table" | "vector";
	onSearch?: (args: {
		query: string;
		mode: "table" | "vector";
		vector?: number[];
		topK?: number;
		where?: Record<string, unknown>;
	}) => Promise<void> | void;
	className?: string;
	tableName?: string;
	children?: React.ReactNode;
}

interface LanceDBContextValue {
	onUpdateItem?: (
		filter: string,
		updates: Record<string, any>,
	) => Promise<void>;
}

const LanceDBContext = React.createContext<LanceDBContextValue>({});

interface TableSettings {
	id: string;
	tableName: string;
	columnVisibility: VisibilityState;
	columnOrder: string[];
	sorting: SortingState;
	pageSize: number;
	columnSizing?: ColumnSizingState;
	density?: "compact" | "comfortable" | "spacious";
}

class SettingsDB extends Dexie {
	tableSettings!: Table<TableSettings>;

	constructor() {
		super("LanceDBExplorerSettings");
		this.version(1).stores({
			tableSettings: "id, tableName",
		});
		this.version(2)
			.stores({
				tableSettings: "id, tableName",
			})
			.upgrade((tx) => {
				return tx
					.table("tableSettings")
					.toCollection()
					.modify((s: any) => {
						if (s.columnSizing == null) s.columnSizing = {};
						if (s.density == null) s.density = "comfortable";
					});
			});
	}
}

const db = new SettingsDB();
const DEFAULT_PAGE_SIZE = 50;
const DEFAULT_DENSITY: "compact" | "comfortable" | "spacious" = "comfortable";

const LanceDBExplorer: React.FC<LanceDBExplorerProps> = ({
	tableName = "table",
	children,
	arrowSchema,
	rows = [],
	total,
	loading = false,
	error = null,
	onPageRequest,
	onUpdateItem,
	onOptimize,
	onDropColumns,
	onAddColumn,
	onAlterColumn,
	onBuildIndex,
	onGetIndices,
	onDropIndex,
	onRefresh,
	pageSizeOptions = [25, 50, 100, 250],
	initialPage = 1,
	initialPageSize,
	initialMode = "table",
	onSearch,
	className,
}) => {
	const [schema, setSchema] = useState<LanceSchema | null>(null);

	const [page, setPage] = useState(initialPage);
	const [pageSize, setPageSize] = useState<number>(
		initialPageSize ?? pageSizeOptions?.[0] ?? DEFAULT_PAGE_SIZE,
	);

	// Track if page/pageSize change came from user interaction vs prop sync
	const isUserInteraction = React.useRef(false);

	// Sync page/pageSize from props when they change (controlled mode)
	useEffect(() => {
		if (page !== initialPage) {
			setPage(initialPage);
		}
	}, [initialPage, page]);

	useEffect(() => {
		if (initialPageSize !== undefined && pageSize !== initialPageSize) {
			setPageSize(initialPageSize);
		}
	}, [initialPageSize, pageSize]);

	// Wrapper to track user-initiated page changes
	const handlePageChange = React.useCallback((newPage: number) => {
		isUserInteraction.current = true;
		setPage(newPage);
	}, []);

	const handlePageSizeChange = React.useCallback((newSize: number) => {
		isUserInteraction.current = true;
		setPageSize(newSize);
		setPage(1); // Reset to first page on size change
	}, []);

	const [columnVisibility, setColumnVisibility] = useState<VisibilityState>({});
	const [columnOrder, setColumnOrder] = useState<ColumnOrderState>([]);
	const [sorting, setSorting] = useState<SortingState>([]);
	const [rowSelection, setRowSelection] = useState({});
	const [columnFilters, setColumnFilters] = useState<ColumnFiltersState>([]);
	const [globalQuery, setGlobalQuery] = useState("");
	const [appliedQuery, setAppliedQuery] = useState("");

	const [lastCount, setLastCount] = useState(0);
	const [fullscreen, setFullscreen] = useState(false);

	const [columnSizing, setColumnSizing] = useState<ColumnSizingState>({});
	const [density, setDensity] = useState<
		"compact" | "comfortable" | "spacious"
	>(DEFAULT_DENSITY);

	const settingsId = `${tableName}_settings`;
	const [settingsLoaded, setSettingsLoaded] = useState(false);
	const [initialPageRequestDone, setInitialPageRequestDone] = useState(false);

	// Stable default for page size (avoid depending on array identity)
	const pageSizeDefault = useMemo(
		() => pageSizeOptions?.[0] ?? DEFAULT_PAGE_SIZE,
		[pageSizeOptions?.[0]],
	);

	// Load settings once on mount or when table changes
	useEffect(() => {
		let cancelled = false;
		setSettingsLoaded(false);
		setInitialPageRequestDone(false);
		const load = async () => {
			try {
				const settings = await db.tableSettings.get(settingsId);
				if (settings && !cancelled) {
					setColumnVisibility(settings.columnVisibility ?? {});
					setColumnOrder(settings.columnOrder ?? []);
					setSorting(settings.sorting ?? []);
					// Only use stored pageSize if no initialPageSize was provided
					if (initialPageSize === undefined) {
						setPageSize(settings.pageSize ?? pageSizeDefault);
					}
					setColumnSizing(settings.columnSizing ?? {});
					setDensity(settings.density ?? DEFAULT_DENSITY);
				}
			} catch (error) {
				console.warn("Failed to load settings:", error);
			} finally {
				if (!cancelled) setSettingsLoaded(true);
			}
		};
		load();
		return () => {
			cancelled = true;
		};
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [settingsId, pageSizeDefault]);

	// Parse schema from Arrow schema when available (separate from settings)
	useEffect(() => {
		if (arrowSchema?.fields?.length) {
			setSchema(arrowToLanceSchema(arrowSchema));
		} else {
			setSchema(null);
		}
	}, [arrowSchema]);

	// Persist settings whenever they change (skip until loaded)
	useEffect(() => {
		if (!settingsLoaded) return;
		db.tableSettings
			.put({
				id: settingsId,
				tableName,
				columnVisibility,
				columnOrder,
				sorting,
				pageSize,
				columnSizing,
				density,
			})
			.catch((error) => console.warn("Failed to save settings:", error));
	}, [
		settingsLoaded,
		settingsId,
		tableName,
		columnVisibility,
		columnOrder,
		sorting,
		pageSize,
		columnSizing,
		density,
	]);

	// Notify parent to fetch when page/pageSize/appliedQuery change (after settings loaded)
	// Only fire callback on user interaction, not on prop sync
	useEffect(() => {
		if (!settingsLoaded) return;
		if (!isUserInteraction.current && initialPageRequestDone) return;

		isUserInteraction.current = false;
		const offset = (page - 1) * pageSize;
		const limit = pageSize;
		onPageRequest?.({ page, pageSize, offset, limit, query: appliedQuery });
		if (!initialPageRequestDone) {
			setInitialPageRequestDone(true);
		}
	}, [page, pageSize, appliedQuery, settingsLoaded, initialPageRequestDone]);

	// Keep inferred schema from rows if no Arrow schema was provided
	useEffect(() => {
		if (!arrowSchema && rows?.length) {
			setSchema((prev) => prev ?? inferSchemaFromRows(rows, tableName));
		}
		setLastCount(rows?.length ?? 0);
	}, [arrowSchema, rows, tableName]);

	const columns = useMemo<ColumnDef<Record<string, any>>[]>(() => {
		if (!schema) return [];
		const base: ColumnDef<Record<string, any>>[] = schema.fields.map((f) =>
			buildColumnForField(f),
		);

		base.unshift({
			id: "select",
			header: ({ table }) => (
				<Checkbox
					checked={table.getIsAllPageRowsSelected()}
					onCheckedChange={(v) => table.toggleAllPageRowsSelected(!!v)}
					aria-label="Select all"
				/>
			),
			cell: ({ row }) => (
				<Checkbox
					checked={row.getIsSelected()}
					onCheckedChange={(v) => row.toggleSelected(!!v)}
					aria-label="Select row"
				/>
			),
			enableSorting: false,
			enableHiding: false,
			size: 28,
		});

		return base;
	}, [schema]);

	const table = useReactTable({
		data: rows ?? [],
		columns,
		state: {
			columnVisibility,
			columnOrder,
			sorting,
			rowSelection,
			columnFilters,
			globalFilter: appliedQuery,
			columnSizing,
			pagination: {
				pageIndex: Math.max(0, page - 1),
				pageSize,
			},
		},
		onColumnVisibilityChange: setColumnVisibility,
		onColumnOrderChange: setColumnOrder,
		onSortingChange: setSorting,
		onRowSelectionChange: setRowSelection,
		onColumnFiltersChange: setColumnFilters,
		onGlobalFilterChange: setAppliedQuery,
		onColumnSizingChange: setColumnSizing,
		onPaginationChange: (updater) => {
			const next =
				typeof updater === "function"
					? updater({ pageIndex: Math.max(0, page - 1), pageSize })
					: updater;
			if (next.pageSize !== pageSize) {
				handlePageSizeChange(next.pageSize);
			} else if (next.pageIndex !== Math.max(0, page - 1)) {
				handlePageChange(next.pageIndex + 1);
			}
		},
		getCoreRowModel: getCoreRowModel(),
		getSortedRowModel: getSortedRowModel(),
		getFilteredRowModel: getFilteredRowModel(),
		manualPagination: true,
		pageCount:
			typeof total === "number" && total > 0
				? Math.ceil((total ?? 0) / pageSize)
				: undefined,
		defaultColumn: { minSize: 80, maxSize: 600 },
		enableColumnResizing: true,
		columnResizeMode: "onChange",
		autoResetPageIndex: false,
	});

	const copySelectedAsCSV = useCallback(() => {
		const rows = table.getSelectedRowModel().rows;
		const cols = table
			.getAllLeafColumns()
			.filter((c) => c.id !== "select")
			.map((c) => c.id);
		const csv = [cols.join(",")].concat(
			rows.map((r) => cols.map((c) => stringifyCSV(r.getValue(c))).join(",")),
		);
		navigator.clipboard.writeText(csv.join("\n"));
	}, [table]);

	const copySelectedAsJSON = useCallback(() => {
		const rows = table.getSelectedRowModel().rows;
		const json = JSON.stringify(
			rows.map((r) => r.original),
			null,
			2,
		);
		navigator.clipboard.writeText(json);
	}, [table]);

	const selectedCount = table.getSelectedRowModel().rows.length;

	const currentFrom = (page - 1) * pageSize + 1;
	const currentTo = Math.min(page * pageSize, total || 0);
	const knowsTotal = typeof total === "number" && total > 0;
	const isLastPage = knowsTotal
		? currentTo >= (total ?? 0)
		: lastCount < pageSize;

	const containerCls = cn(
		"flex h-full w-full flex-col gap-3",
		fullscreen && "fixed inset-0 z-[60] bg-background p-4",
		className,
	);

	// dnd-kit sensors for better UX (avoid accidental drags)
	const sensors = useSensors(
		useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
	);

	const contextValue = useMemo<LanceDBContextValue>(
		() => ({ onUpdateItem }),
		[onUpdateItem],
	);

	return (
		<LanceDBContext.Provider value={contextValue}>
			<div className={containerCls}>
				<div className="flex items-center gap-2 flex-shrink-0">
					<Database className="h-5 w-5" />
					<div className="text-sm text-muted-foreground">{tableName}</div>
					<Separator orientation="vertical" className="mx-1" />
					<div className="ml-auto flex items-center gap-2">
						<DensityToggle value={density} onChange={setDensity} />
						<Button
							variant="outline"
							size="sm"
							onClick={() => setFullscreen((v) => !v)}
							title={fullscreen ? "Exit fullscreen" : "Fullscreen"}
						>
							{fullscreen ? (
								<>
									<Minimize2 className="h-4 w-4 mr-2" /> Exit
								</>
							) : (
								<>
									<Maximize2 className="h-4 w-4 mr-2" /> Fullscreen
								</>
							)}
						</Button>
						<SchemaDialog
							schema={schema}
							tableName={tableName}
							onDropColumns={onDropColumns}
							onAddColumn={onAddColumn}
							onAlterColumn={onAlterColumn}
							onBuildIndex={onBuildIndex}
							onGetIndices={onGetIndices}
							onDropIndex={onDropIndex}
						/>
						<DatabaseActionsDropdown
							onOptimize={onOptimize}
							onRefresh={onRefresh}
						/>
						<ColumnVisibilityDropdown
							columns={table.getAllLeafColumns().map((c: any) => ({
								id: c.id,
								canHide: c.getCanHide(),
							}))}
							visibility={columnVisibility}
							onChange={setColumnVisibility}
						/>
						{children}
					</div>
				</div>

				<Toolbar
					value={globalQuery}
					onValueChange={setGlobalQuery}
					onSearch={() => {
						setAppliedQuery(globalQuery);
						handlePageChange(1);
						onSearch?.({ query: globalQuery, mode: initialMode });
					}}
					onReset={() => {
						setGlobalQuery("");
						setAppliedQuery("");
						handlePageChange(1);
						onSearch?.({ query: "", mode: initialMode });
					}}
				/>

				<div className="flex flex-col flex-1 min-h-0 min-w-0 rounded-xl border bg-card">
					<div className="flex-1 w-full overflow-auto min-h-0">
						<DataTable className="w-full">
							<TableHeader className="sticky top-0 bg-card z-10">
								{table.getHeaderGroups().map((headerGroup) => {
									const headers = headerGroup.headers.filter(
										(h) => h.column.getIsVisible?.() !== false,
									);
									const draggableItems = headers
										.map((h) => h.id)
										.filter((id) => id !== "select"); // keep 'select' anchored

									const handleDragEnd = (e: DragEndEvent) => {
										const { active, over } = e;
										if (!over || active.id === over.id) return;

										const visibleDraggable = headers
											.map((h) => h.id)
											.filter((id) => id !== "select");

										if (
											!visibleDraggable.includes(String(active.id)) ||
											!visibleDraggable.includes(String(over.id))
										)
											return;

										const from = visibleDraggable.indexOf(String(active.id));
										const to = visibleDraggable.indexOf(String(over.id));
										const newVisible = arrayMove(visibleDraggable, from, to);

										const allIds = table
											.getAllLeafColumns()
											.map((c: any) => c.id);
										const prevOrder = [
											...table.getState().columnOrder,
											...allIds.filter(
												(id: string) =>
													!table.getState().columnOrder.includes(id),
											),
										];

										const setToReorder = new Set(visibleDraggable);
										const pool = [...newVisible];
										const nextOrder = prevOrder.map((id) =>
											setToReorder.has(id) ? pool.shift()! : id,
										);

										table.setColumnOrder(nextOrder);
									};

									return (
										<DndContext
											key={headerGroup.id}
											sensors={sensors}
											collisionDetection={closestCenter}
											onDragEnd={handleDragEnd}
										>
											<SortableContext
												items={draggableItems}
												strategy={horizontalListSortingStrategy}
											>
												<TableRow>
													{headerGroup.headers.map((header) => (
														<SortableHeaderCell
															key={header.id}
															header={header}
															table={table}
															isDraggable={header.id !== "select"}
														/>
													))}
												</TableRow>
											</SortableContext>
										</DndContext>
									);
								})}
							</TableHeader>
							<TableBody>
								{loading ? (
									<TableRow>
										<TableCell
											colSpan={columns.length}
											className="h-24 text-center text-muted-foreground"
										>
											Loading…
										</TableCell>
									</TableRow>
								) : table.getRowModel().rows?.length ? (
									table.getRowModel().rows.map((row) => (
										<TableRow
											key={row.id}
											className={cn(
												"hover:bg-muted/30",
												"odd:bg-muted/10",
												density === "compact"
													? "h-8"
													: density === "spacious"
														? "h-14"
														: "h-10",
											)}
										>
											{row.getVisibleCells().map((cell) => (
												<TableCell
													key={cell.id}
													className={cn(
														density === "compact"
															? "py-1"
															: density === "spacious"
																? "py-3"
																: "py-2",
													)}
												>
													{flexRender(
														cell.column.columnDef.cell,
														cell.getContext(),
													)}
												</TableCell>
											))}
										</TableRow>
									))
								) : (
									<TableRow>
										<TableCell
											colSpan={columns.length}
											className="h-24 text-center text-muted-foreground"
										>
											<div className="flex w-full h-full items-center justify-center">
												<div className="flex items-center gap-2">
													<Info className="h-4 w-4" /> No results.
												</div>
											</div>
										</TableCell>
									</TableRow>
								)}
							</TableBody>
						</DataTable>
					</div>

					<div className="flex items-center justify-between px-3 py-2 border-t bg-muted/20 flex-shrink-0">
						<div className="text-xs text-muted-foreground">
							{knowsTotal ? (
								<>
									Showing <b>{currentFrom}</b>–<b>{currentTo}</b> of{" "}
									<b>{(total ?? 0).toLocaleString()}</b>
								</>
							) : (
								<>—</>
							)}
							{selectedCount > 0 && (
								<Badge variant="secondary" className="ml-2">
									{selectedCount} selected
								</Badge>
							)}
						</div>
						<div className="flex items-center gap-2">
							<Select
								value={String(pageSize)}
								onValueChange={(v) => handlePageSizeChange(Number(v))}
							>
								<SelectTrigger className="h-8 w-[120px]">
									<SelectValue placeholder="Page size" />
								</SelectTrigger>
								<SelectContent>
									{pageSizeOptions.map((n) => (
										<SelectItem key={n} value={String(n)}>
											{n} / page
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<Button
								variant="outline"
								size="sm"
								onClick={() => handlePageChange(Math.max(1, page - 1))}
								disabled={page === 1}
							>
								Prev
							</Button>
							<div className="text-sm w-14 text-center">{page}</div>
							<Button
								variant="outline"
								size="sm"
								onClick={() => handlePageChange(page + 1)}
								disabled={isLastPage}
							>
								Next
							</Button>

							<DropdownMenu>
								<DropdownMenuTrigger asChild>
									<Button variant="outline" size="sm">
										<MoreHorizontal className="h-4 w-4" />
									</Button>
								</DropdownMenuTrigger>
								<DropdownMenuContent align="end">
									<DropdownMenuItem onClick={copySelectedAsCSV}>
										<Download className="h-4 w-4 mr-2" /> Copy selected as CSV
									</DropdownMenuItem>
									<DropdownMenuItem onClick={copySelectedAsJSON}>
										<ClipboardList className="h-4 w-4 mr-2" /> Copy selected as
										JSON
									</DropdownMenuItem>
									<DropdownMenuSeparator />
									<DropdownMenuItem
										onClick={() => {
											setColumnVisibility({});
											setColumnOrder([]);
											setSorting([]);
											setColumnSizing({});
										}}
									>
										<RefreshCcw className="h-4 w-4 mr-2" /> Reset layout
									</DropdownMenuItem>
								</DropdownMenuContent>
							</DropdownMenu>
						</div>
					</div>
				</div>

				{error && (
					<div className="text-sm text-destructive flex items-center gap-2 flex-shrink-0">
						<Info className="h-4 w-4" /> {error}
					</div>
				)}
			</div>
		</LanceDBContext.Provider>
	);
};

export default LanceDBExplorer;

// Replace DraggableTableHead with dnd-kit powered SortableHeaderCell
const SortableHeaderCell: React.FC<{
	header: any;
	table: any;
	isDraggable?: boolean;
}> = ({ header, table, isDraggable = true }) => {
	if (!header.column.getIsVisible?.()) return null;

	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
		isOver,
	} = useSortable({
		id: header.id,
		disabled: !isDraggable,
	});

	const style: React.CSSProperties = {
		transform: transform ? CSS.Translate.toString(transform) : undefined,
		transition,
	};

	return (
		<TableHead
			ref={setNodeRef}
			style={{ width: header.getSize(), ...style }}
			className={cn(
				"relative group border-b bg-muted/30 select-none",
				isDragging && "opacity-50 ring-1 ring-primary/40",
				isOver && "bg-primary/10",
			)}
		>
			<div className="flex items-center gap-1">
				{isDraggable && (
					<span
						title="Drag to reorder"
						className="inline-flex h-4 w-4 items-center justify-center text-muted-foreground cursor-grab active:cursor-grabbing"
						{...attributes}
						{...listeners}
						onClick={(e) => e.stopPropagation()}
					>
						<GripVertical className="h-3 w-3 opacity-70" />
					</span>
				)}
				{header.isPlaceholder ? null : (
					<div
						className={cn(
							"flex select-none items-center gap-1 flex-1",
							header.column.getCanSort() && "cursor-pointer",
						)}
						onClick={header.column.getToggleSortingHandler()}
					>
						{flexRender(header.column.columnDef.header, header.getContext())}
						{{ asc: "↑", desc: "↓" }[header.column.getIsSorted() as string] ??
							null}
					</div>
				)}
			</div>
		</TableHead>
	);
};

const ColumnVisibilityDropdown: React.FC<{
	columns: { id: string; canHide: boolean }[];
	visibility: VisibilityState;
	onChange: React.Dispatch<React.SetStateAction<VisibilityState>>;
}> = ({ columns, visibility, onChange }) => {
	const [query, setQuery] = useState("");

	const isVisible = useCallback(
		(id: string) => (visibility[id] ?? true) === true,
		[visibility],
	);

	const setVisible = useCallback(
		(id: string, v: boolean) => {
			onChange((prev) => ({ ...prev, [id]: v }));
		},
		[onChange],
	);

	const filtered = useMemo(
		() =>
			columns
				.filter((c) => c.canHide)
				.filter(
					(c) => !query || c.id.toLowerCase().includes(query.toLowerCase()),
				),
		[columns, query],
	);

	const showAll = useCallback(() => {
		onChange((prev) => {
			const next = { ...prev };
			filtered.forEach((c) => {
				next[c.id] = true;
			});
			return next;
		});
	}, [filtered, onChange]);

	const hideAll = useCallback(() => {
		onChange((prev) => {
			const next = { ...prev };
			filtered.forEach((c) => {
				next[c.id] = false;
			});
			return next;
		});
	}, [filtered, onChange]);

	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button variant="outline" size="sm">
					<Columns3 className="h-4 w-4 mr-2" /> Columns
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-64">
				<DropdownMenuLabel className="flex items-center justify-between">
					<span>Toggle columns</span>
					<SlidersHorizontal className="h-4 w-4 text-muted-foreground" />
				</DropdownMenuLabel>
				<div className="px-2 pb-2">
					<Input
						value={query}
						onChange={(e) => setQuery(e.target.value)}
						placeholder="Filter columns…"
						className="h-8"
					/>
				</div>
				<div className="px-2 pb-2 flex items-center justify-between gap-2">
					<div className="flex flex-row items-center gap-2">
						<Button size="sm" variant="outline" onClick={showAll}>
							All
						</Button>
						<Button size="sm" variant="outline" onClick={hideAll}>
							None
						</Button>
					</div>
					<span className="text-xs text-muted-foreground">
						{filtered.length}/{columns.filter((c) => c.canHide).length}
					</span>
				</div>
				<DropdownMenuSeparator />
				<div className="max-h-64 overflow-auto pr-1">
					{filtered.map((c) => (
						<DropdownMenuCheckboxItem
							key={c.id}
							className="capitalize"
							checked={isVisible(c.id)}
							onCheckedChange={(v) => setVisible(c.id, !!v)}
							onSelect={(e) => e.preventDefault()}
						>
							{c.id}
						</DropdownMenuCheckboxItem>
					))}
					{filtered.length === 0 && (
						<div className="px-2 py-2 text-xs text-muted-foreground">
							No columns.
						</div>
					)}
				</div>
			</DropdownMenuContent>
		</DropdownMenu>
	);
};

const DensityToggle: React.FC<{
	value: "compact" | "comfortable" | "spacious";
	onChange: (v: "compact" | "comfortable" | "spacious") => void;
}> = ({ value, onChange }) => {
	const label =
		value === "compact"
			? "Compact"
			: value === "spacious"
				? "Spacious"
				: "Comfort";
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button variant="outline" size="sm" title="Row density">
					<SlidersHorizontal className="h-4 w-4 mr-2" /> {label}
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-40">
				<DropdownMenuItem onClick={() => onChange("compact")}>
					Compact
				</DropdownMenuItem>
				<DropdownMenuItem onClick={() => onChange("comfortable")}>
					Comfortable
				</DropdownMenuItem>
				<DropdownMenuItem onClick={() => onChange("spacious")}>
					Spacious
				</DropdownMenuItem>
			</DropdownMenuContent>
		</DropdownMenu>
	);
};

const DateCell: React.FC<{ value: any; onClick: () => void }> = ({
	value,
	onClick,
}) => {
	const date = new Date(value);
	const isValid = !isNaN(date.getTime());

	if (!isValid) {
		return (
			<Button variant="ghost" size="sm" className="h-6 px-2" onClick={onClick}>
				{String(value)}
			</Button>
		);
	}

	const relativeTime = getRelativeTime(date);
	const fullDate = date.toLocaleString(undefined, {
		year: "numeric",
		month: "long",
		day: "numeric",
		hour: "2-digit",
		minute: "2-digit",
		second: "2-digit",
	});

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button
					variant="ghost"
					size="sm"
					className="h-6 px-2 tabular-nums justify-start gap-1.5"
					onClick={onClick}
				>
					<Clock className="h-3 w-3 text-muted-foreground" />
					{relativeTime}
				</Button>
			</TooltipTrigger>
			<TooltipContent side="bottom" className="text-xs">
				{fullDate}
			</TooltipContent>
		</Tooltip>
	);
};

const getRelativeTime = (date: Date): string => {
	const now = new Date();
	const diff = now.getTime() - date.getTime();
	const absDiff = Math.abs(diff);
	const isFuture = diff < 0;

	const seconds = Math.floor(absDiff / 1000);
	const minutes = Math.floor(seconds / 60);
	const hours = Math.floor(minutes / 60);
	const days = Math.floor(hours / 24);

	if (seconds < 60) return isFuture ? "in a moment" : "just now";
	if (minutes < 60) return isFuture ? `in ${minutes}m` : `${minutes}m ago`;
	if (hours < 24) return isFuture ? `in ${hours}h` : `${hours}h ago`;
	if (days < 7) return isFuture ? `in ${days}d` : `${days}d ago`;

	// For older dates, show abbreviated date
	return date.toLocaleDateString(undefined, {
		month: "short",
		day: "numeric",
		year: date.getFullYear() !== now.getFullYear() ? "numeric" : undefined,
	});
};

const buildColumnForField = (
	f: LanceField,
): ColumnDef<Record<string, any>> => ({
	id: f.name,
	accessorFn: (row: Record<string, any>) => row[f.name],
	header: f.name,
	enableSorting: true,
	enableColumnFilter: true,
	cell: ({ getValue, row }) => (
		<Cell value={getValue()} field={f} rowData={row.original} />
	),
});

const Cell: React.FC<{
	value: any;
	field: LanceField;
	rowData: Record<string, any>;
}> = ({ value, field, rowData }) => {
	const { onUpdateItem } = useContext(LanceDBContext);
	const [dialogOpen, setDialogOpen] = useState(false);
	const [editValue, setEditValue] = useState("");

	const openDialog = useCallback(() => {
		setEditValue(safeStringify(value, 2));
		setDialogOpen(true);
	}, [value]);

	const renderCellPreviewButton = () => {
		if (value == null) {
			return (
				<Button
					variant="ghost"
					size="sm"
					className="h-6 px-2 text-muted-foreground"
					onClick={openDialog}
				>
					NULL
				</Button>
			);
		}

		switch (field.kind) {
			case "boolean":
				return (
					<Button
						variant="ghost"
						size="sm"
						className="h-6 px-2"
						onClick={openDialog}
					>
						<Checkbox checked={!!value} disabled aria-label="bool" />
					</Button>
				);
			case "number":
				return (
					<Button
						variant="ghost"
						size="sm"
						className="h-6 px-2 tabular-nums justify-start"
						onClick={openDialog}
					>
						{value}
					</Button>
				);
			case "date":
				return <DateCell value={value} onClick={openDialog} />;
			case "vector": {
				const arr = ensureNumericArray(value);
				const dims = field.dims ?? arr.length;
				return (
					<Button
						variant="ghost"
						size="sm"
						className="h-6 px-2 flex items-center gap-2"
						onClick={openDialog}
					>
						<Badge variant="outline">{dims}d</Badge>
						<Sparkline data={arr.slice(0, 64)} />
					</Button>
				);
			}
			case "array":
			case "object":
			case "unknown":
				return (
					<Button
						variant="ghost"
						size="sm"
						className="h-6 px-2"
						onClick={openDialog}
					>
						<ListTree className="h-3 w-3 mr-1" /> View
					</Button>
				);
			default: {
				// Check if string value looks like an ISO date
				if (typeof value === "string" && isISODateString(value)) {
					return <DateCell value={value} onClick={openDialog} />;
				}
				return (
					<Button
						variant="ghost"
						size="sm"
						className="h-6 px-2 justify-start max-w-[200px] truncate"
						onClick={openDialog}
					>
						{String(value)}
					</Button>
				);
			}
		}
	};

	return (
		<>
			{renderCellPreviewButton()}
			<CellViewDialog
				open={dialogOpen}
				onOpenChange={setDialogOpen}
				value={value}
				valueStr={editValue}
				onValueChange={setEditValue}
				field={field}
				rowData={rowData}
				onUpdateItem={onUpdateItem}
			/>
		</>
	);
};

const CellViewDialog: React.FC<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	value: any;
	valueStr: string;
	onValueChange: (value: string) => void;
	field: LanceField;
	rowData: Record<string, any>;
	onUpdateItem?: (
		filter: string,
		updates: Record<string, any>,
	) => Promise<void>;
}> = ({
	open,
	onOpenChange,
	value,
	valueStr,
	onValueChange,
	field,
	rowData,
	onUpdateItem,
}) => {
	const [localValue, setLocalValue] = useState(valueStr);
	const [isEditing, setIsEditing] = useState(false);
	const [saving, setSaving] = useState(false);

	useEffect(() => {
		setLocalValue(valueStr);
	}, [valueStr]);

	useEffect(() => {
		if (!open) {
			setIsEditing(false);
		}
	}, [open]);

	const handleSave = useCallback(async () => {
		if (!onUpdateItem) return;

		setSaving(true);
		try {
			let parsedValue: any;
			try {
				parsedValue = JSON.parse(localValue);
			} catch {
				parsedValue = localValue;
			}

			// Build filter based on row data - try to use id or _rowid if available
			const idField = rowData.id ?? rowData._rowid ?? rowData._id;
			let filter: string;
			if (idField !== undefined) {
				const idKey =
					"id" in rowData ? "id" : "_rowid" in rowData ? "_rowid" : "_id";
				filter =
					typeof idField === "string"
						? `${idKey} = '${idField}'`
						: `${idKey} = ${idField}`;
			} else {
				// Fallback: build filter from all primitive fields
				const conditions = Object.entries(rowData)
					.filter(
						([, v]) =>
							typeof v === "string" ||
							typeof v === "number" ||
							typeof v === "boolean",
					)
					.slice(0, 3)
					.map(([k, v]) =>
						typeof v === "string" ? `${k} = '${v}'` : `${k} = ${v}`,
					);
				filter = conditions.join(" AND ");
			}

			await onUpdateItem(filter, { [field.name]: parsedValue });
			onValueChange(localValue);
			setIsEditing(false);
			onOpenChange(false);
		} finally {
			setSaving(false);
		}
	}, [
		localValue,
		onValueChange,
		onOpenChange,
		onUpdateItem,
		rowData,
		field.name,
	]);

	const PreviewContent = useMemo(() => {
		if (value == null) {
			return <div className="text-sm text-muted-foreground">NULL</div>;
		}

		switch (field.kind) {
			case "boolean":
				return (
					<div className="flex items-center gap-2">
						<Checkbox checked={!!value} disabled aria-label="bool" />
						<span className="text-sm text-muted-foreground">
							{String(!!value)}
						</span>
					</div>
				);
			case "number":
				return <code className="text-sm">{String(value)}</code>;
			case "date":
				return <code className="text-sm">{formatDate(value)}</code>;
			case "vector": {
				const arr = ensureNumericArray(value);
				const dims = field.dims ?? arr.length;
				return (
					<div className="flex items-center gap-3">
						<Badge variant="outline">{dims}d</Badge>
						<Sparkline data={arr.slice(0, 128)} />
					</div>
				);
			}
			case "array":
			case "object":
			case "unknown":
				return (
					<pre className="whitespace-pre-wrap break-all text-xs font-mono bg-muted rounded-md p-3 overflow-x-auto max-w-full">
						{safeStringify(value, 2)}
					</pre>
				);
			default: {
				if (typeof value === "string") {
					return <TextEditor initialContent={value} isMarkdown={true} />;
				}
				return (
					<pre className="whitespace-pre-wrap break-all text-sm bg-muted/50 rounded-md p-3 overflow-x-auto max-w-full">
						{String(value)}
					</pre>
				);
			}
		}
	}, [field.kind, field.dims, value]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-7xl w-full min-w-fit overflow-hidden">
				<DialogHeader>
					<DialogTitle className="flex items-center justify-between">
						<span>
							{isEditing ? `Edit ${field.name}` : `Preview ${field.name}`}
						</span>
						<div className="flex items-center gap-3 mr-4">
							<div className="flex items-center gap-2">
								<Label
									htmlFor="edit-switch"
									className="text-xs text-muted-foreground"
								>
									Edit
								</Label>
								<Switch
									id="edit-switch"
									checked={isEditing}
									onCheckedChange={setIsEditing}
								/>
							</div>
							<Badge variant="outline">{field.kind}</Badge>
						</div>
					</DialogTitle>
				</DialogHeader>
				<div className="space-y-4 overflow-hidden">
					{!isEditing ? (
						<div className="max-h-[60vh] overflow-auto pr-1">
							{PreviewContent}
						</div>
					) : (
						<Textarea
							value={localValue}
							onChange={(e) => setLocalValue(e.target.value)}
							className="min-h-[300px] max-h-[60vh] font-mono text-sm"
							placeholder="Enter value..."
						/>
					)}
					<div className="flex justify-end gap-2">
						<Button variant="outline" onClick={() => onOpenChange(false)}>
							<X className="h-4 w-4 mr-2" /> Close
						</Button>
						{isEditing && (
							<Button onClick={handleSave} disabled={saving || !onUpdateItem}>
								<Save className="h-4 w-4 mr-2" />{" "}
								{saving ? "Saving..." : "Save"}
							</Button>
						)}
					</div>
				</div>
			</DialogContent>
		</Dialog>
	);
};

const Sparkline: React.FC<{ data: number[] }> = ({ data }) => {
	const ref = React.useRef<HTMLCanvasElement | null>(null);

	useEffect(() => {
		const canvas = ref.current;
		if (!canvas) return;

		const dpr = window.devicePixelRatio || 1;
		const w = 80,
			h = 20;
		canvas.width = w * dpr;
		canvas.height = h * dpr;
		canvas.style.width = `${w}px`;
		canvas.style.height = `${h}px`;

		const ctx = canvas.getContext("2d");
		if (!ctx) return;

		ctx.scale(dpr, dpr);
		ctx.clearRect(0, 0, w, h);
		ctx.lineWidth = 1;
		ctx.beginPath();

		if (data.length === 0) return;

		const min = Math.min(...data);
		const max = Math.max(...data);
		const range = max - min || 1;

		data.forEach((v, i) => {
			const x = (i / Math.max(1, data.length - 1)) * (w - 2) + 1;
			const y = h - 1 - ((v - min) / range) * (h - 2);
			if (i === 0) ctx.moveTo(x, y);
			else ctx.lineTo(x, y);
		});

		ctx.strokeStyle = getComputedStyle(
			document.documentElement,
		).getPropertyValue("--primary");
		ctx.stroke();
	}, [data]);

	return <canvas ref={ref} aria-label="sparkline" />;
};

const Toolbar: React.FC<{
	value: string;
	onValueChange: (v: string) => void;
	onSearch: () => void;
	onReset: () => void;
}> = ({ value, onValueChange, onSearch, onReset }) => (
	<div className="flex flex-wrap items-center gap-2 flex-shrink-0">
		<div className="relative w-full sm:w-[380px]">
			<Search className="absolute left-2 top-2.5 h-4 w-4 text-muted-foreground" />
			<Input
				className="pl-8"
				placeholder="Search…"
				value={value}
				onChange={(e) => onValueChange(e.target.value)}
				onKeyDown={(e) => {
					if (e.key === "Enter") onSearch();
				}}
			/>
		</div>
		<Button variant="outline" size="sm" onClick={onSearch}>
			Apply
		</Button>
		<Button variant="ghost" size="sm" onClick={onReset}>
			Reset
		</Button>
	</div>
);

const DatabaseActionsDropdown: React.FC<{
	onOptimize?: (keepVersions?: boolean) => Promise<void>;
	onRefresh?: () => void;
}> = ({ onOptimize, onRefresh }) => {
	const [optimizing, setOptimizing] = useState(false);

	const handleOptimize = async (keepVersions: boolean) => {
		if (!onOptimize) return;
		setOptimizing(true);
		try {
			await onOptimize(keepVersions);
		} finally {
			setOptimizing(false);
		}
	};

	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button variant="outline" size="sm" title="Database Actions">
					<Wrench className="h-4 w-4 mr-2" /> Actions
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-56">
				<DropdownMenuLabel>Database Operations</DropdownMenuLabel>
				<DropdownMenuSeparator />
				{onRefresh && (
					<DropdownMenuItem onClick={onRefresh}>
						<RefreshCcw className="h-4 w-4 mr-2" /> Refresh Data
					</DropdownMenuItem>
				)}
				{onOptimize && (
					<>
						<DropdownMenuItem
							onClick={() => handleOptimize(true)}
							disabled={optimizing}
						>
							<Zap className="h-4 w-4 mr-2" />
							{optimizing ? "Optimizing..." : "Optimize (Keep Versions)"}
						</DropdownMenuItem>
						<DropdownMenuItem
							onClick={() => handleOptimize(false)}
							disabled={optimizing}
						>
							<Zap className="h-4 w-4 mr-2" />
							{optimizing ? "Optimizing..." : "Optimize (Compact)"}
						</DropdownMenuItem>
					</>
				)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
};

const SchemaDialog: React.FC<{
	schema: LanceSchema | null;
	tableName?: string;
	onDropColumns?: (columns: string[]) => Promise<void>;
	onAddColumn?: (name: string, sqlExpression: string) => Promise<void>;
	onAlterColumn?: (column: string, nullable: boolean) => Promise<void>;
	onBuildIndex?: (column: string, indexType: string) => Promise<void>;
	onGetIndices?: () => Promise<IIndexConfig[]>;
	onDropIndex?: (indexName: string) => Promise<void>;
}> = ({
	schema,
	tableName,
	onDropColumns,
	onAddColumn,
	onAlterColumn,
	onBuildIndex,
	onGetIndices,
	onDropIndex,
}) => {
	const [open, setOpen] = useState(false);
	const [activeTab, setActiveTab] = useState<"schema" | "indices" | "add">(
		"schema",
	);
	const [indices, setIndices] = useState<IIndexConfig[]>([]);
	const [loadingIndices, setLoadingIndices] = useState(false);
	const [newColumnName, setNewColumnName] = useState("");
	const [newColumnType, setNewColumnType] = useState("string");
	const [newColumnDefault, setNewColumnDefault] = useState("");
	const [indexColumn, setIndexColumn] = useState("");
	const [indexType, setIndexType] = useState("auto");
	const [processing, setProcessing] = useState(false);

	const loadIndices = useCallback(async () => {
		if (!onGetIndices) return;
		setLoadingIndices(true);
		try {
			const result = await onGetIndices();
			setIndices(result);
		} finally {
			setLoadingIndices(false);
		}
	}, [onGetIndices]);

	useEffect(() => {
		if (open && activeTab === "indices" && onGetIndices) {
			loadIndices();
		}
	}, [open, activeTab, loadIndices, onGetIndices]);

	const handleDropColumn = async (columnName: string) => {
		if (!onDropColumns) return;
		setProcessing(true);
		try {
			await onDropColumns([columnName]);
		} finally {
			setProcessing(false);
		}
	};

	const handleDropIndex = async (indexName: string) => {
		if (!onDropIndex) return;
		setProcessing(true);
		try {
			await onDropIndex(indexName);
			await loadIndices();
		} finally {
			setProcessing(false);
		}
	};

	const handleAddColumn = async () => {
		if (!onAddColumn || !newColumnName) return;
		const expression = buildAddColumnExpression(newColumnType, newColumnDefault);
		if (!expression) return;
		setProcessing(true);
		try {
			await onAddColumn(newColumnName, expression);
			setNewColumnName("");
			setNewColumnDefault("");
		} finally {
			setProcessing(false);
		}
	};

	const handleBuildIndex = async () => {
		if (!onBuildIndex || !indexColumn) return;
		setProcessing(true);
		try {
			await onBuildIndex(indexColumn, indexType);
			await loadIndices();
			setIndexColumn("");
		} finally {
			setProcessing(false);
		}
	};

	const hasSchemaOps = onDropColumns || onAddColumn || onAlterColumn;
	const hasIndexOps = onBuildIndex || onGetIndices;

	return (
		<>
			<Button variant="outline" size="sm" onClick={() => setOpen(true)}>
				<Settings className="h-4 w-4 mr-2" /> Schema
			</Button>
			<Dialog open={open} onOpenChange={setOpen}>
				<DialogContent className="w-full max-w-lg max-h-[80vh] overflow-y-auto">
					<DialogHeader>
						<DialogTitle>Table: {tableName}</DialogTitle>
					</DialogHeader>

					{(hasSchemaOps || hasIndexOps) && (
						<div className="flex gap-2 border-b pb-2">
							<Button
								variant={activeTab === "schema" ? "default" : "ghost"}
								size="sm"
								onClick={() => setActiveTab("schema")}
							>
								Schema
							</Button>
							{hasIndexOps && (
								<Button
									variant={activeTab === "indices" ? "default" : "ghost"}
									size="sm"
									onClick={() => setActiveTab("indices")}
								>
									Indices
								</Button>
							)}
							{hasSchemaOps && (
								<Button
									variant={activeTab === "add" ? "default" : "ghost"}
									size="sm"
									onClick={() => setActiveTab("add")}
								>
									Modify
								</Button>
							)}
						</div>
					)}

					{activeTab === "schema" && (
						<>
							{schema ? (
								<ScrollArea className="max-h-[50vh]">
									<div className="space-y-2 pr-2">
										{schema.fields.map((f) => (
											<div
												key={f.name}
												className="flex items-center justify-between gap-3 py-2 px-2 rounded-md hover:bg-muted/50"
											>
												<div className="flex-1">
													<div className="font-medium text-sm">{f.name}</div>
													<div className="text-xs text-muted-foreground">
														{describeField(f)}
													</div>
												</div>
												{f.kind === "vector" && (
													<Badge variant="secondary">
														{f.dims ?? "?"} dims
													</Badge>
												)}
												{onDropColumns && (
													<Button
														variant="ghost"
														size="sm"
														className="h-7 px-2 text-destructive hover:text-destructive"
														onClick={() => handleDropColumn(f.name)}
														disabled={processing}
													>
														<Trash2 className="h-3 w-3" />
													</Button>
												)}
											</div>
										))}
									</div>
								</ScrollArea>
							) : (
								<div className="text-sm text-muted-foreground py-4">
									No schema loaded yet.
								</div>
							)}
						</>
					)}

					{activeTab === "indices" && (
						<div className="space-y-4">
							<div className="space-y-2">
								<Label>Current Indices</Label>
								{loadingIndices ? (
									<div className="text-sm text-muted-foreground">
										Loading...
									</div>
								) : indices.length > 0 ? (
									<ScrollArea className="max-h-[30vh]">
										<div className="space-y-2 pr-2">
											{indices.map((idx) => (
												<div
													key={idx.name}
													className="flex items-center justify-between gap-2 p-2 rounded-md bg-muted/50"
												>
													<div className="min-w-0 flex-1">
														<div className="font-medium text-sm truncate">
															{idx.name}
														</div>
														<div className="text-xs text-muted-foreground truncate">
															{idx.index_type} on {idx.columns.join(", ")}
														</div>
													</div>
													{onDropIndex && (
														<Button
															variant="ghost"
															size="sm"
															className="h-7 px-2 text-destructive hover:text-destructive flex-shrink-0"
															onClick={() => handleDropIndex(idx.name)}
															disabled={processing}
														>
															<Trash2 className="h-3 w-3" />
														</Button>
													)}
												</div>
											))}
										</div>
									</ScrollArea>
								) : (
									<div className="text-sm text-muted-foreground">
										No indices found.
									</div>
								)}
							</div>

							{onBuildIndex && schema && (
								<div className="space-y-3 border-t pt-4">
									<Label>Create New Index</Label>
									<div className="flex gap-2">
										<Select value={indexColumn} onValueChange={setIndexColumn}>
											<SelectTrigger className="flex-1">
												<SelectValue placeholder="Select column" />
											</SelectTrigger>
											<SelectContent>
												{schema.fields.map((f) => (
													<SelectItem key={f.name} value={f.name}>
														{f.name}
													</SelectItem>
												))}
											</SelectContent>
										</Select>
										<IndexTypeSelect
											value={indexType}
											onChange={setIndexType}
											className="w-32"
										/>
										<Button
											onClick={handleBuildIndex}
											disabled={!indexColumn || processing}
										>
											{processing ? "Building..." : "Build"}
										</Button>
									</div>
								</div>
							)}
						</div>
					)}

					{activeTab === "add" && (
						<div className="space-y-4 flex flex-col">
							{onAddColumn && (
								<div className="space-y-3 flex-shrink-0">
									<Label>Add New Column</Label>
									<div className="grid gap-2 sm:grid-cols-[1fr_150px]">
										<Input
											placeholder="Column name"
											value={newColumnName}
											onChange={(e) => setNewColumnName(e.target.value)}
										/>
										<ColumnTypeSelect
											value={newColumnType}
											onChange={setNewColumnType}
											groups={EDIT_COLUMN_TYPE_GROUPS}
										/>
									</div>
									<Input
										placeholder="Default value (optional — leave empty for NULL)"
										value={newColumnDefault}
										onChange={(e) => setNewColumnDefault(e.target.value)}
									/>
									<div className="text-xs text-muted-foreground">
										New columns are added as nullable. Leave the default empty
										to backfill existing rows with NULL, or provide a typed
										default value.
									</div>
									<Button
										onClick={handleAddColumn}
										disabled={!newColumnName || processing}
										className="w-full"
									>
										{processing ? "Adding..." : "Add Column"}
									</Button>
								</div>
							)}

							{onAlterColumn && schema && (
								<div className="space-y-3 border-t pt-4 flex-1 min-h-0 flex flex-col">
									<Label className="flex-shrink-0">Make Column Nullable</Label>
									<div className="text-xs text-muted-foreground mb-2 flex-shrink-0">
										Note: LanceDB only supports making columns nullable, not the
										reverse.
									</div>
									<ScrollArea className="flex-1 min-h-0">
										<div className="space-y-1 pr-2">
											{schema.fields.map((f) => (
												<div
													key={f.name}
													className="flex items-center justify-between gap-2 p-2 rounded-md hover:bg-muted/50"
												>
													<div className="min-w-0 flex-1">
														<span className="text-sm truncate block">
															{f.name}
														</span>
														<span className="text-xs text-muted-foreground">
															{f.nullable ? "Nullable" : "Not Nullable"}
														</span>
													</div>
													{!f.nullable && (
														<Button
															variant="outline"
															size="sm"
															className="flex-shrink-0"
															onClick={() => onAlterColumn(f.name, true)}
															disabled={processing}
														>
															Make Nullable
														</Button>
													)}
												</div>
											))}
										</div>
									</ScrollArea>
								</div>
							)}
						</div>
					)}
				</DialogContent>
			</Dialog>
		</>
	);
};

export const arrowToLanceSchema = (arrow: ArrowSchemaJSON): LanceSchema => ({
	table:
		typeof arrow?.metadata?.["name"] === "string"
			? String(arrow.metadata["name"])
			: "table",
	fields: (arrow?.fields ?? []).map(arrowFieldToLance),
});

const arrowFieldToLance = (f: any): LanceField => {
	const name = String(f?.name ?? "");
	const dt = f?.data_type;
	const nullable = f?.nullable ?? true;

	if (typeof dt === "string") {
		return { name, kind: arrowPrimitiveToKind(dt), nullable };
	}

	if (dt && typeof dt === "object") {
		if (dt.FixedSizeList) {
			const [child, size] = dt.FixedSizeList as [any, number];
			const childType = child?.data_type;
			if (
				childType === "Float32" ||
				childType === "Float64" ||
				childType === "Float16"
			) {
				return {
					name,
					kind: "vector",
					dims: Number(size) || undefined,
					nullable,
				};
			}
			return {
				name,
				kind: "array",
				items:
					typeof childType === "string"
						? arrowPrimitiveToKind(childType)
						: "unknown",
				nullable,
			};
		}
		if (dt.List) {
			const [child] = dt.List as [any];
			const childType = child?.data_type;
			return {
				name,
				kind: "array",
				items:
					typeof childType === "string"
						? arrowPrimitiveToKind(childType)
						: "unknown",
				nullable,
			};
		}
		if (dt.Struct) {
			return { name, kind: "object", nullable };
		}
		if (dt.Map) {
			return { name, kind: "object", nullable };
		}
	}

	return { name, kind: "unknown", nullable };
};

const arrowPrimitiveToKind = (dt: string): LanceFieldKind => {
	switch (dt) {
		case "Utf8":
		case "LargeUtf8":
		case "Binary":
		case "LargeBinary":
			return "string";
		case "Bool":
			return "boolean";
		case "Int8":
		case "Int16":
		case "Int32":
		case "Int64":
		case "UInt8":
		case "UInt16":
		case "UInt32":
		case "UInt64":
		case "Float16":
		case "Float32":
		case "Float64":
			return "number";
		case "Date32":
		case "Date64":
		case "Timestamp":
			return "date";
		default:
			return "unknown";
	}
};

const stringifyCSV = (v: any) => {
	if (v == null) return "";
	const s = typeof v === "string" ? v : safeStringify(v);
	if (s.includes(",") || s.includes("\n") || s.includes('"')) {
		return '"' + s.replaceAll('"', '""') + '"';
	}
	return s;
};

const safeStringify = (v: any, space?: number) => {
	try {
		return JSON.stringify(v, null, space);
	} catch {
		return String(v);
	}
};

const ensureNumericArray = (v: any): number[] => {
	if (Array.isArray(v)) return v.map(Number).filter((n) => Number.isFinite(n));
	if (typeof v === "string") return parseVector(v) ?? [];
	return [];
};

const parseVector = (text: string | undefined): number[] | undefined => {
	if (!text) return undefined;
	try {
		if (text.trim().startsWith("[")) {
			const arr = JSON.parse(text);
			return Array.isArray(arr)
				? arr.map(Number).filter((n) => Number.isFinite(n))
				: undefined;
		}
		return text
			.split(/[\s,]+/)
			.map((s) => s.trim())
			.filter(Boolean)
			.map(Number)
			.filter((n) => Number.isFinite(n));
	} catch {
		return undefined;
	}
};

const formatDate = (v: any): string => {
	try {
		const d = new Date(v);
		if (isNaN(d.getTime())) return String(v);

		const now = new Date();
		const diff = now.getTime() - d.getTime();
		const absDiff = Math.abs(diff);

		// For very recent times (< 1 minute), show "just now"
		if (absDiff < 60_000) return "just now";

		// For recent times (< 1 hour), show minutes
		if (absDiff < 3_600_000) {
			const mins = Math.floor(absDiff / 60_000);
			return diff > 0 ? `${mins}m ago` : `in ${mins}m`;
		}

		// For today, show time only
		if (d.toDateString() === now.toDateString()) {
			return d.toLocaleTimeString(undefined, {
				hour: "2-digit",
				minute: "2-digit",
			});
		}

		// For this year, show month and day
		if (d.getFullYear() === now.getFullYear()) {
			return d.toLocaleDateString(undefined, {
				month: "short",
				day: "numeric",
				hour: "2-digit",
				minute: "2-digit",
			});
		}

		// For older dates, show full date
		return d.toLocaleDateString(undefined, {
			year: "numeric",
			month: "short",
			day: "numeric",
		});
	} catch {
		return String(v);
	}
};

const describeField = (f: LanceField): string => {
	switch (f.kind) {
		case "vector":
			return `${f.kind}${f.dims ? `(${f.dims})` : ""}`;
		case "array":
			return `array<${
				typeof f.items === "string"
					? f.items
					: ((f.items as any)?.kind ?? "unknown")
			}>`;
		default:
			return f.kind;
	}
};

// Infer schema from values when no Arrow schema is provided
const inferSchemaFromRows = (
	rows: Record<string, any>[],
	tableName?: string,
): LanceSchema => {
	const sample = rows.slice(0, 100);
	const keys = new Set<string>();
	sample.forEach((r) => Object.keys(r ?? {}).forEach((k) => keys.add(k)));
	const fields: LanceField[] = Array.from(keys).map((name) =>
		inferField(
			name,
			sample.map((r) => r?.[name]),
		),
	);
	return { table: tableName ?? "table", fields };
};

const inferField = (name: string, values: any[]): LanceField => {
	const nonNull = values.filter((v) => v !== null && v !== undefined);
	if (!nonNull.length) return { name, kind: "unknown" };

	// Vector: numeric arrays with consistent length
	if (
		nonNull.every(
			(v) =>
				Array.isArray(v) && v.every((n: any) => Number.isFinite(Number(n))),
		)
	) {
		const len = nonNull[0].length;
		const consistent = nonNull.every((v) => v.length === len);
		if (len > 0 && consistent) return { name, kind: "vector", dims: len };
	}

	// Array
	if (nonNull.every((v) => Array.isArray(v))) {
		const flat = (nonNull as any[]).flatMap((a: any[]) => a);
		const itemKind = inferPrimitiveKind(flat);
		return { name, kind: "array", items: itemKind };
	}

	// Boolean
	if (nonNull.every((v) => typeof v === "boolean"))
		return { name, kind: "boolean" };

	// Number
	if (nonNull.every((v) => typeof v === "number" && Number.isFinite(v)))
		return { name, kind: "number" };

	// Date (Date objects or ISO-like strings)
	if (
		nonNull.every(
			(v) => v instanceof Date || (typeof v === "string" && isISODateString(v)),
		)
	) {
		return { name, kind: "date" };
	}

	// Object
	if (
		nonNull.every(
			(v) => typeof v === "object" && v !== null && !Array.isArray(v),
		)
	) {
		return { name, kind: "object" };
	}

	// String
	if (nonNull.every((v) => typeof v === "string"))
		return { name, kind: "string" };

	return { name, kind: "unknown" };
};

const inferPrimitiveKind = (vals: any[]): LanceFieldKind | LanceField => {
	const nonNull = vals.filter((v) => v !== null && v !== undefined);
	if (!nonNull.length) return "unknown";
	if (nonNull.every((v) => typeof v === "boolean")) return "boolean";
	if (nonNull.every((v) => typeof v === "number" && Number.isFinite(v)))
		return "number";
	if (nonNull.every((v) => typeof v === "string")) return "string";
	if (
		nonNull.every(
			(v) => v instanceof Date || (typeof v === "string" && isISODateString(v)),
		)
	)
		return "date";
	if (
		nonNull.every(
			(v) =>
				Array.isArray(v) && v.every((n: any) => Number.isFinite(Number(n))),
		)
	) {
		const len = (nonNull[0] as any[]).length;
		const consistent = nonNull.every((v) => (v as any[]).length === len);
		if (len > 0 && consistent) return { name: "", kind: "vector", dims: len }; // unused name
	}
	return "unknown";
};

const isISODateString = (s: string): boolean => {
	// Match ISO 8601 formats: 2025-12-17, 2025-12-17T11:47:30, 2025-12-17T11:47:30.475130Z
	return /^\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:?\d{2})?)?$/.test(
		s,
	);
};
