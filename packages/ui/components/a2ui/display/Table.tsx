"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
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
	verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
	ArrowDownIcon,
	ArrowUpDownIcon,
	ArrowUpIcon,
	CheckIcon,
	ColumnsIcon,
	CopyIcon,
	DownloadIcon,
	EyeIcon,
	EyeOffIcon,
	FilterIcon,
	GripVerticalIcon,
	SearchIcon,
} from "lucide-react";
import * as React from "react";

import { cn } from "../../../lib/utils";
import {
	Button,
	Checkbox,
	Input,
	Popover,
	PopoverContent,
	PopoverTrigger,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "../../ui/index";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { toEventContextValue } from "../event-context";
import type {
	BoundValue,
	TableCellComponent,
	TableColumn,
	TableComponent,
	TableRowComponent,
} from "../types";

/** Table never dispatched actions before, so no surface can have meant to
 * subscribe to these through `*` or `actions[0]`. */
const EXACT_ONLY = { legacyFallback: false, wildcardFallback: false };

type SortDirection = "asc" | "desc" | null;

interface ColumnFilter {
	column: number;
	value: string;
}

/**
 * A rendered row paired with the record it came from. Search, filter, sort and
 * pagination all reorder rows, so the position on screen says nothing about
 * the record — events must report `index` and `source`, not the visual slot.
 */
interface TableRowModel {
	index: number;
	cells: string[];
	source: Record<string, unknown>;
}

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function tableToCSV(headers: string[], rows: string[][]): string {
	const escapeCell = (cell: string) => {
		if (cell.includes(",") || cell.includes('"') || cell.includes("\n")) {
			return `"${cell.replace(/"/g, '""')}"`;
		}
		return cell;
	};
	return [
		headers.map(escapeCell).join(","),
		...rows.map((row) => row.map(escapeCell).join(",")),
	].join("\n");
}

function tableToMarkdown(headers: string[], rows: string[][]): string {
	if (headers.length === 0) return "";
	const separator = headers.map(() => "---");
	return [
		`| ${headers.join(" | ")} |`,
		`| ${separator.join(" | ")} |`,
		...rows.map((row) => `| ${row.join(" | ")} |`),
	].join("\n");
}

async function downloadCSV(
	headers: string[],
	rows: string[][],
	filename = "table.csv",
) {
	const csv = tableToCSV(headers, rows);

	if (typeof window !== "undefined" && "__TAURI__" in window) {
		try {
			const { save } = await import("@tauri-apps/plugin-dialog");
			const { writeTextFile } = await import("@tauri-apps/plugin-fs");

			const filePath = await save({
				canCreateDirectories: true,
				title: i18next.t('saveCsv', 'Save CSV'),
				defaultPath: filename,
				filters: [{ name: "CSV", extensions: ["csv"] }],
			});

			if (filePath) {
				await writeTextFile(filePath, csv);
				return;
			}
			return;
		} catch (e) {
			console.warn("Tauri save failed, using fallback:", e);
		}
	}

	const dataUrl = `data:text/csv;charset=utf-8,${encodeURIComponent(csv)}`;
	const link = document.createElement("a");
	link.href = dataUrl;
	link.download = filename;
	link.style.display = "none";
	document.body.appendChild(link);
	link.click();
	document.body.removeChild(link);
}

function SortableColumnItem({
	colIdx,
	header,
	isHidden,
	onToggleVisibility,
}: {
	colIdx: number;
	header: string;
	isHidden: boolean;
	onToggleVisibility: () => void;
}) {
	const { t } = useTranslation("common");
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
	} = useSortable({
		id: `col-${colIdx}`,
	});

	const style: React.CSSProperties = {
		transform: transform ? CSS.Transform.toString(transform) : undefined,
		transition,
	};

	return (
		<div
			ref={setNodeRef}
			style={style}
			className={cn(
				"flex items-center gap-2 px-1 py-0.5 rounded hover:bg-muted/50",
				isDragging && "opacity-50 bg-muted",
			)}
		>
			<GripVerticalIcon
				className="h-3 w-3 text-muted-foreground shrink-0 cursor-grab"
				{...attributes}
				{...listeners}
			/>
			<Checkbox
				id={`col-visibility-${colIdx}`}
				checked={!isHidden}
				onCheckedChange={onToggleVisibility}
				className="h-3.5 w-3.5"
			/>
			<label
				htmlFor={`col-visibility-${colIdx}`}
				className="text-xs flex-1 truncate cursor-pointer"
				title={header || t('columnVal', 'Column {{val}}', { val: colIdx + 1 })}
			>
				{header || t('columnVal', 'Column {{val}}', { val: colIdx + 1 })}
			</label>
			{isHidden ? (
				<EyeOffIcon className="h-3 w-3 text-muted-foreground shrink-0" />
			) : (
				<EyeIcon className="h-3 w-3 text-muted-foreground shrink-0" />
			)}
		</div>
	);
}

function SortableHeaderCell({
	colIdx,
	header,
	sortColumn,
	sortDirection,
	onSort,
	sortable,
	align,
}: {
	colIdx: number;
	header: string;
	sortColumn: number | null;
	sortDirection: SortDirection;
	onSort: (colIdx: number) => void;
	sortable: boolean;
	align?: string;
}) {
	const { t } = useTranslation("common");
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
	} = useSortable({
		id: `col-${colIdx}`,
	});

	const style: React.CSSProperties = {
		transform: transform ? CSS.Transform.toString(transform) : undefined,
		transition,
	};

	const alignClass =
		align === "center"
			? "justify-center"
			: align === "right"
				? "justify-end"
				: "justify-start";

	return (
		<th
			ref={setNodeRef}
			style={style}
			className={cn(
				"font-semibold px-2 py-1.5 hover:bg-muted/50 transition-colors whitespace-nowrap select-none",
				isDragging && "opacity-50 bg-muted",
			)}
		>
			<div className={cn("flex items-center gap-1", alignClass)}>
				<GripVerticalIcon
					className="h-3 w-3 text-muted-foreground/50 shrink-0 cursor-grab"
					{...attributes}
					{...listeners}
				/>
				<span
					className={cn("truncate max-w-[200px]", sortable && "cursor-pointer")}
					onClick={() => sortable && onSort(colIdx)}
				>
					{header || t('columnVal', 'Column {{val}}', { val: colIdx + 1 })}
				</span>
				{sortable &&
					(sortColumn === colIdx ? (
						sortDirection === "asc" ? (
							<ArrowUpIcon className="h-3 w-3 text-primary shrink-0" />
						) : sortDirection === "desc" ? (
							<ArrowDownIcon className="h-3 w-3 text-primary shrink-0" />
						) : (
							<ArrowUpDownIcon className="h-3 w-3 text-muted-foreground/50 shrink-0" />
						)
					) : (
						<ArrowUpDownIcon className="h-3 w-3 text-muted-foreground/50 opacity-0 group-hover/table:opacity-100 shrink-0" />
					))}
			</div>
		</th>
	);
}

export function A2UITable({
	component,
	style,
	componentId,
}: ComponentProps<TableComponent>) {
	const { t } = useTranslation("common");
	const triggerEvent = useComponentEventTrigger(componentId);
	const columns = useResolved<TableColumn[]>(component.columns) ?? [];
	const data = useResolved<Record<string, unknown>[]>(component.data) ?? [];
	const caption = useResolved<string>(component.caption);
	const striped = useResolved<boolean>(component.striped) ?? false;
	const bordered = useResolved<boolean>(component.bordered) ?? false;
	const hoverable = useResolved<boolean>(component.hoverable) ?? true;
	const compact = useResolved<boolean>(component.compact) ?? false;
	const stickyHeader = useResolved<boolean>(component.stickyHeader) ?? false;
	const sortable = useResolved<boolean>(component.sortable) ?? true;
	const searchable = useResolved<boolean>(component.searchable) ?? true;
	const paginated = useResolved<boolean>(component.paginated) ?? false;
	const pageSize = useResolved<number>(component.pageSize) ?? 10;
	const selectable = useResolved<boolean>(component.selectable) ?? false;

	const [copied, setCopied] = React.useState(false);
	const [searchQuery, setSearchQuery] = React.useState("");
	const [sortColumn, setSortColumn] = React.useState<number | null>(null);
	const [sortDirection, setSortDirection] = React.useState<SortDirection>(null);
	const [columnFilters, setColumnFilters] = React.useState<ColumnFilter[]>([]);
	const [showFilters, setShowFilters] = React.useState(false);
	const [showColumns, setShowColumns] = React.useState(false);
	const [currentPage, setCurrentPage] = React.useState(0);
	const [selectedRows, setSelectedRows] = React.useState<Set<number>>(
		new Set(),
	);

	const headers = columns.map((col) => {
		if (typeof col.header === "string") return col.header;
		if (typeof col.header === "object" && "literalString" in col.header) {
			return col.header.literalString;
		}
		return col.id;
	});

	const [columnOrder, setColumnOrder] = React.useState<number[]>(() =>
		columns.map((_, i) => i),
	);
	const [hiddenColumns, setHiddenColumns] = React.useState<Set<number>>(
		() => new Set(),
	);

	const sensors = useSensors(
		useSensor(PointerSensor, {
			activationConstraint: { distance: 5 },
		}),
	);

	React.useEffect(() => {
		setColumnOrder(columns.map((_, i) => i));
		setHiddenColumns(new Set());
	}, [columns.length]);

	const visibleColumns = React.useMemo(
		() => columnOrder.filter((idx) => !hiddenColumns.has(idx)),
		[columnOrder, hiddenColumns],
	);

	const columnIds = React.useMemo(
		() => columnOrder.map((idx) => `col-${idx}`),
		[columnOrder],
	);

	const getAccessor = (col: TableColumn): string => {
		if (!col.accessor) return col.id;
		if (typeof col.accessor === "string") return col.accessor;
		if (typeof col.accessor === "object" && "literalString" in col.accessor) {
			return col.accessor.literalString;
		}
		if (typeof col.accessor === "object" && "path" in col.accessor) {
			return col.accessor.path;
		}
		return col.id;
	};

	const rows: TableRowModel[] = React.useMemo(() => {
		return data.map((row, index) => ({
			index,
			source: row,
			cells: columns.map((col) => {
				const accessor = getAccessor(col);
				const value = row[accessor];
				return value != null ? String(value) : "";
			}),
		}));
	}, [data, columns]);

	const searchedRows = React.useMemo(() => {
		if (!searchable || !searchQuery.trim()) return rows;
		const query = searchQuery.toLowerCase();
		return rows.filter((row) =>
			row.cells.some((cell) => cell.toLowerCase().includes(query)),
		);
	}, [rows, searchQuery, searchable]);

	const filteredRows = React.useMemo(() => {
		if (columnFilters.length === 0) return searchedRows;
		return searchedRows.filter((row) =>
			columnFilters.every((filter) => {
				const cellValue = row.cells[filter.column]?.toLowerCase() || "";
				return cellValue.includes(filter.value.toLowerCase());
			}),
		);
	}, [searchedRows, columnFilters]);

	const sortedRows = React.useMemo(() => {
		if (!sortable || sortColumn === null || sortDirection === null)
			return filteredRows;
		return [...filteredRows].sort((a, b) => {
			const aVal = a.cells[sortColumn] || "";
			const bVal = b.cells[sortColumn] || "";

			const aNum = Number.parseFloat(aVal);
			const bNum = Number.parseFloat(bVal);
			if (!Number.isNaN(aNum) && !Number.isNaN(bNum)) {
				return sortDirection === "asc" ? aNum - bNum : bNum - aNum;
			}

			const comparison = aVal.localeCompare(bVal);
			return sortDirection === "asc" ? comparison : -comparison;
		});
	}, [filteredRows, sortColumn, sortDirection, sortable]);

	const paginatedRows = React.useMemo(() => {
		if (!paginated) return sortedRows;
		const start = currentPage * pageSize;
		return sortedRows.slice(start, start + pageSize);
	}, [sortedRows, paginated, currentPage, pageSize]);

	const totalPages = React.useMemo(
		() => Math.ceil(sortedRows.length / pageSize),
		[sortedRows.length, pageSize],
	);

	const handleCopy = React.useCallback(
		(format: "csv" | "markdown") => {
			const visibleHeaders = visibleColumns.map((idx) => headers[idx]);
			const visibleRows = sortedRows.map((row) =>
				visibleColumns.map((idx) => row.cells[idx]),
			);
			const text =
				format === "csv"
					? tableToCSV(visibleHeaders, visibleRows)
					: tableToMarkdown(visibleHeaders, visibleRows);
			navigator.clipboard.writeText(text);
			setCopied(true);
			setTimeout(() => setCopied(false), 2000);
		},
		[headers, sortedRows, visibleColumns],
	);

	const handleDownload = React.useCallback(() => {
		const visibleHeaders = visibleColumns.map((idx) => headers[idx]);
		const visibleRows = sortedRows.map((row) =>
			visibleColumns.map((idx) => row.cells[idx]),
		);
		downloadCSV(visibleHeaders, visibleRows);
	}, [headers, sortedRows, visibleColumns]);

	const handleColumnSort = React.useCallback(
		(columnIndex: number) => {
			const nextDirection: SortDirection =
				sortColumn !== columnIndex
					? "asc"
					: sortDirection === "asc"
						? "desc"
						: sortDirection === "desc"
							? null
							: "asc";

			setSortColumn(columnIndex);
			setSortDirection(nextDirection);
			void triggerEvent(
				"sortChange",
				component,
				{
					columnIndex,
					columnId: columns[columnIndex]?.id ?? null,
					direction: nextDirection,
				},
				EXACT_ONLY,
			);
		},
		[sortColumn, sortDirection, columns, component, triggerEvent],
	);

	const handleColumnFilter = React.useCallback(
		(columnIndex: number, value: string) => {
			setColumnFilters((prev) => {
				const existing = prev.findIndex((f) => f.column === columnIndex);
				if (!value.trim()) {
					return prev.filter((f) => f.column !== columnIndex);
				}
				if (existing >= 0) {
					const updated = [...prev];
					updated[existing] = { column: columnIndex, value };
					return updated;
				}
				return [...prev, { column: columnIndex, value }];
			});
		},
		[],
	);

	const toggleColumnVisibility = React.useCallback(
		(columnIndex: number) => {
			setHiddenColumns((prev) => {
				const next = new Set(prev);
				if (next.has(columnIndex)) {
					next.delete(columnIndex);
				} else {
					if (next.size >= columns.length - 1) return prev;
					next.add(columnIndex);
				}
				return next;
			});
		},
		[columns.length],
	);

	const handleColumnDragEnd = React.useCallback((event: DragEndEvent) => {
		const { active, over } = event;
		if (!over || active.id === over.id) return;

		const activeIdx = Number.parseInt(
			(active.id as string).replace("col-", ""),
			10,
		);
		const overIdx = Number.parseInt(
			(over.id as string).replace("col-", ""),
			10,
		);

		setColumnOrder((prev) => {
			const oldIndex = prev.indexOf(activeIdx);
			const newIndex = prev.indexOf(overIdx);
			return arrayMove(prev, oldIndex, newIndex);
		});
	}, []);

	const resetColumns = React.useCallback(() => {
		setColumnOrder(columns.map((_, i) => i));
		setHiddenColumns(new Set());
	}, [columns]);

	const clearFilters = React.useCallback(() => {
		setSearchQuery("");
		setColumnFilters([]);
		setSortColumn(null);
		setSortDirection(null);
		setCurrentPage(0);
	}, []);

	const applySelection = React.useCallback(
		(next: Set<number>) => {
			setSelectedRows(next);
			const selected = [...next].sort((a, b) => a - b);
			void triggerEvent(
				"selectionChange",
				component,
				{
					selectedIndexes: selected,
					selectedRows: selected.map((index) =>
						toEventContextValue(rows[index]?.source ?? null),
					),
				},
				EXACT_ONLY,
			);
		},
		[component, rows, triggerEvent],
	);

	const toggleRowSelection = React.useCallback(
		(rowIndex: number) => {
			const next = new Set(selectedRows);
			if (next.has(rowIndex)) {
				next.delete(rowIndex);
			} else {
				next.add(rowIndex);
			}
			applySelection(next);
		},
		[applySelection, selectedRows],
	);

	const allPageRowsSelected =
		paginatedRows.length > 0 &&
		paginatedRows.every((row) => selectedRows.has(row.index));

	const toggleAllRows = React.useCallback(() => {
		applySelection(
			allPageRowsSelected
				? new Set()
				: new Set(paginatedRows.map((row) => row.index)),
		);
	}, [allPageRowsSelected, applySelection, paginatedRows]);

	const handleRowClick = React.useCallback(
		(row: TableRowModel) => {
			void triggerEvent(
				"rowClick",
				component,
				{
					rowIndex: row.index,
					row: toEventContextValue(row.source),
					cells: row.cells,
				},
				EXACT_ONLY,
			);
		},
		[component, triggerEvent],
	);

	const handleCellClick = React.useCallback(
		(row: TableRowModel, columnIndex: number) => {
			void triggerEvent(
				"cellClick",
				component,
				{
					rowIndex: row.index,
					columnIndex,
					columnId: columns[columnIndex]?.id ?? null,
					value: row.cells[columnIndex] ?? null,
					row: toEventContextValue(row.source),
				},
				EXACT_ONLY,
			);
		},
		[columns, component, triggerEvent],
	);

	// One handler on the row keeps the row keyboard-reachable; the column comes
	// from the cell that was actually hit, so the selection column cannot shift
	// the index the board receives.
	const handleRowActivate = React.useCallback(
		(row: TableRowModel, target: EventTarget | null, fromPointer: boolean) => {
			handleRowClick(row);
			if (!fromPointer) return;

			const cell =
				target instanceof Element
					? target.closest("[data-column-index]")
					: null;
			const columnIndex = Number.parseInt(
				cell?.getAttribute("data-column-index") ?? "",
				10,
			);
			if (Number.isInteger(columnIndex)) handleCellClick(row, columnIndex);
		},
		[handleCellClick, handleRowClick],
	);

	const rowsInteractive = Boolean(
		component.eventHandlers?.rowClick?.length ||
			component.eventHandlers?.cellClick?.length,
	);

	const hasActiveFilters =
		searchQuery.trim() || columnFilters.length > 0 || sortColumn !== null;
	const hasColumnChanges =
		hiddenColumns.size > 0 || !columnOrder.every((col, idx) => col === idx);

	const getColumnAlign = (colIdx: number): string | undefined => {
		const col = columns[colIdx];
		if (!col?.align) return undefined;
		if (typeof col.align === "string") return col.align;
		if (typeof col.align === "object" && "literalString" in col.align) {
			return col.align.literalString;
		}
		return undefined;
	};

	const getColumnSortable = (colIdx: number): boolean => {
		const col = columns[colIdx];
		if (!col?.sortable) return sortable;
		if (typeof col.sortable === "object" && "literalBool" in col.sortable) {
			return col.sortable.literalBool;
		}
		return sortable;
	};

	return (
		<div
			className={cn("group/table relative", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{(searchable || sortable) && (
				<div className="flex items-center gap-1 mb-2 flex-wrap opacity-0 transition-opacity group-hover/table:opacity-100 focus-within:opacity-100">
					{searchable && (
						<div className="relative">
							<SearchIcon className="absolute left-2 top-1/2 -translate-y-1/2 h-3 w-3 text-muted-foreground" />
							<Input
								type="text"
								placeholder="Search..."
								value={searchQuery}
								onChange={(e) => setSearchQuery(e.target.value)}
								className="h-7 w-40 pl-7 text-xs"
							/>
						</div>
					)}

					<Popover open={showFilters} onOpenChange={setShowFilters}>
						<PopoverTrigger asChild>
							<Button
								variant={columnFilters.length > 0 ? "secondary" : "ghost"}
								size="sm"
								className="h-7 px-2 text-xs"
							>
								<FilterIcon className="h-3 w-3 mr-1" />
								{t('filter', 'Filter')}
								{columnFilters.length > 0 && (
									<span className="ml-1 bg-primary text-primary-foreground rounded-full px-1.5 text-[10px]">
										{columnFilters.length}
									</span>
								)}
							</Button>
						</PopoverTrigger>
						<PopoverContent className="w-64 p-3" align="start">
							<div className="space-y-2">
								<div className="text-xs font-medium text-muted-foreground mb-2">
									{t('filterByColumn', 'Filter by column')}
								</div>
								{headers.map((header, idx) => (
									<div key={idx} className="flex items-center gap-2">
										<span className="text-xs w-20 truncate" title={header}>
											{header || t('colVal', 'Col {{val}}', { val: idx + 1 })}
										</span>
										<Input
											type="text"
											placeholder="Contains..."
											value={
												columnFilters.find((f) => f.column === idx)?.value || ""
											}
											onChange={(e) => handleColumnFilter(idx, e.target.value)}
											className="h-6 text-xs flex-1"
										/>
									</div>
								))}
							</div>
						</PopoverContent>
					</Popover>

					<Popover open={showColumns} onOpenChange={setShowColumns}>
						<PopoverTrigger asChild>
							<Button
								variant={hasColumnChanges ? "secondary" : "ghost"}
								size="sm"
								className="h-7 px-2 text-xs"
							>
								<ColumnsIcon className="h-3 w-3 mr-1" />
								{t('columns', 'Columns')}
								{hiddenColumns.size > 0 && (
									<span className="ml-1 bg-muted-foreground text-muted rounded-full px-1.5 text-[10px]">
										{columns.length - hiddenColumns.size}/{columns.length}
									</span>
								)}
							</Button>
						</PopoverTrigger>
						<PopoverContent className="w-56 p-3" align="start">
							<div className="space-y-1">
								<div className="flex items-center justify-between mb-2">
									<span className="text-xs font-medium text-muted-foreground">
										{t('showhideReorder', 'Show/hide & reorder')}
									</span>
									{hasColumnChanges && (
										<Button
											variant="ghost"
											size="sm"
											className="h-5 px-1 text-[10px]"
											onClick={resetColumns}
										>
											{t('reset', 'Reset')}
										</Button>
									)}
								</div>
								<DndContext
									sensors={sensors}
									collisionDetection={closestCenter}
									onDragEnd={handleColumnDragEnd}
								>
									<SortableContext
										items={columnIds}
										strategy={verticalListSortingStrategy}
									>
										{columnOrder.map((colIdx) => (
											<SortableColumnItem
												key={colIdx}
												colIdx={colIdx}
												header={headers[colIdx]}
												isHidden={hiddenColumns.has(colIdx)}
												onToggleVisibility={() =>
													toggleColumnVisibility(colIdx)
												}
											/>
										))}
									</SortableContext>
								</DndContext>
							</div>
						</PopoverContent>
					</Popover>

					<div className="flex-1" />

					{hasActiveFilters && (
						<Button
							variant="ghost"
							size="sm"
							className="h-7 px-2 text-xs text-muted-foreground"
							onClick={clearFilters}
						>
							{t('clearFilters', 'Clear filters')}
						</Button>
					)}

					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="sm"
								className="h-7 px-2"
								onClick={() => handleCopy("csv")}
							>
								{copied ? (
									<CheckIcon className="h-3 w-3 text-green-500" />
								) : (
									<CopyIcon className="h-3 w-3" />
								)}
							</Button>
						</TooltipTrigger>
						<TooltipContent>{t('copyAsCsv', 'Copy as CSV')}</TooltipContent>
					</Tooltip>

					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="sm"
								className="h-7 px-2"
								onClick={handleDownload}
							>
								<DownloadIcon className="h-3 w-3" />
							</Button>
						</TooltipTrigger>
						<TooltipContent>{t('downloadCsv', 'Download CSV')}</TooltipContent>
					</Tooltip>
				</div>
			)}

			<div className="overflow-x-auto">
				<table
					className={cn(
						"w-full text-sm",
						bordered && "border border-border",
						compact ? "text-xs" : "text-sm",
					)}
				>
					{caption && (
						<caption className="text-muted-foreground text-sm py-2">
							{caption}
						</caption>
					)}
					<thead
						className={cn(stickyHeader && "sticky top-0 bg-background z-10")}
					>
						<DndContext
							sensors={sensors}
							collisionDetection={closestCenter}
							onDragEnd={handleColumnDragEnd}
						>
							<SortableContext
								items={visibleColumns.map((idx) => `col-${idx}`)}
								strategy={horizontalListSortingStrategy}
							>
								<tr className={cn(bordered && "border-b border-border")}>
									{selectable && (
										<th className="w-8 px-2 py-1.5">
											<Checkbox
												checked={allPageRowsSelected}
												onCheckedChange={toggleAllRows}
												className="h-3.5 w-3.5"
											/>
										</th>
									)}
									{visibleColumns.map((colIdx) => (
										<SortableHeaderCell
											key={colIdx}
											colIdx={colIdx}
											header={headers[colIdx]}
											sortColumn={sortColumn}
											sortDirection={sortDirection}
											onSort={handleColumnSort}
											sortable={getColumnSortable(colIdx)}
											align={getColumnAlign(colIdx)}
										/>
									))}
								</tr>
							</SortableContext>
						</DndContext>
					</thead>
					<tbody>
						{paginatedRows.map((row, rowIdx) => (
							<tr
								key={row.index}
								className={cn(
									bordered && "border-b border-border",
									striped && rowIdx % 2 === 1 && "bg-muted/50",
									hoverable && "hover:bg-muted/30 transition-colors",
									selectable && selectedRows.has(row.index) && "bg-primary/10",
									rowsInteractive && "cursor-pointer",
								)}
								tabIndex={rowsInteractive ? 0 : undefined}
								onClick={
									rowsInteractive
										? (event) => handleRowActivate(row, event.target, true)
										: undefined
								}
								onKeyDown={
									rowsInteractive
										? (event) => {
												if (event.key !== "Enter" && event.key !== " ") return;
												event.preventDefault();
												handleRowActivate(row, event.target, false);
											}
										: undefined
								}
							>
								{selectable && (
									<td className="w-8 px-2 py-1.5">
										<Checkbox
											checked={selectedRows.has(row.index)}
											onCheckedChange={() => toggleRowSelection(row.index)}
											className="h-3.5 w-3.5"
											onClick={(event) => event.stopPropagation()}
										/>
									</td>
								)}
								{visibleColumns.map((colIdx) => {
									const align = getColumnAlign(colIdx);
									const alignClass =
										align === "center"
											? "text-center"
											: align === "right"
												? "text-right"
												: "text-left";
									return (
										<td
											key={colIdx}
											data-column-index={colIdx}
											className={cn(
												"px-2 py-1.5",
												alignClass,
												compact && "py-1",
											)}
										>
											{row.cells[colIdx]}
										</td>
									);
								})}
							</tr>
						))}
						{paginatedRows.length === 0 && (
							<tr>
								<td
									colSpan={visibleColumns.length + (selectable ? 1 : 0)}
									className="text-center py-8 text-muted-foreground"
								>
									{t('noDataAvailable', 'No data available')}
								</td>
							</tr>
						)}
					</tbody>
				</table>
			</div>

			{paginated && totalPages > 1 && (
				<div className="flex items-center justify-between mt-2 text-xs text-muted-foreground">
					<span>
						{t('showing', 'Showing')} {currentPage * pageSize + 1}-
						{Math.min((currentPage + 1) * pageSize, sortedRows.length)} of{" "}
						{sortedRows.length}
					</span>
					<div className="flex items-center gap-1">
						<Button
							variant="outline"
							size="sm"
							className="h-6 px-2 text-xs"
							disabled={currentPage === 0}
							onClick={() => setCurrentPage((p) => p - 1)}
						>
							{t('previous', 'Previous')}
						</Button>
						<span className="px-2">
							{t('pageOfTotal', 'Page {{page}} of {{total}}', { page: currentPage + 1, total: totalPages })}
						</span>
						<Button
							variant="outline"
							size="sm"
							className="h-6 px-2 text-xs"
							disabled={currentPage >= totalPages - 1}
							onClick={() => setCurrentPage((p) => p + 1)}
						>
							{t('next', 'Next')}
						</Button>
					</div>
				</div>
			)}
		</div>
	);
}

export function A2UITableRow({
	component,
	style,
}: ComponentProps<TableRowComponent>) {
	const cells = useResolved<string[]>(component.cells) ?? [];
	const selected = useResolved<boolean>(component.selected) ?? false;
	const disabled = useResolved<boolean>(component.disabled) ?? false;

	return (
		<tr
			className={cn(
				resolveStyle(style),
				selected && "bg-primary/10",
				disabled && "opacity-50 pointer-events-none",
			)}
			style={resolveInlineStyle(style)}
		>
			{cells.map((cell, idx) => (
				<td key={idx} className="px-2 py-1.5">
					{cell}
				</td>
			))}
		</tr>
	);
}

export function A2UITableCell({
	component,
	style,
}: ComponentProps<TableCellComponent>) {
	const content = useResolved<string>(component.content);
	const isHeader = useResolved<boolean>(component.isHeader) ?? false;
	const colSpan = useResolved<number>(component.colSpan);
	const rowSpan = useResolved<number>(component.rowSpan);
	const align = useResolved<string>(component.align);

	const alignClass =
		align === "center"
			? "text-center"
			: align === "right"
				? "text-right"
				: "text-left";
	const Tag = isHeader ? "th" : "td";

	return (
		<Tag
			colSpan={colSpan}
			rowSpan={rowSpan}
			className={cn(
				"px-2 py-1.5",
				alignClass,
				isHeader && "font-semibold",
				resolveStyle(style),
			)}
			style={resolveInlineStyle(style)}
		>
			{content}
		</Tag>
	);
}
