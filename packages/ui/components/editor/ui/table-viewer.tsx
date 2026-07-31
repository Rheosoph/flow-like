"use client";

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
	WrapTextIcon,
	XIcon,
} from "lucide-react";
import * as React from "react";

import { cn, sanitizeImageUrl } from "../../../lib/utils";
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

interface TableViewerProps {
	data: string[][];
	children: React.ReactNode;
	className?: string;
}

type SortDirection = "asc" | "desc" | null;

interface ColumnFilter {
	column: number;
	value: string;
}

function tableToCSV(data: string[][]): string {
	return data
		.map((row) =>
			row
				.map((cell) => {
					if (cell.includes(",") || cell.includes('"') || cell.includes("\n")) {
						return `"${cell.replace(/"/g, '""')}"`;
					}
					return cell;
				})
				.join(","),
		)
		.join("\n");
}

function tableToMarkdown(data: string[][]): string {
	if (data.length === 0) return "";
	const header = data[0];
	const separator = header.map(() => "---");
	const body = data.slice(1);
	return [
		`| ${header.join(" | ")} |`,
		`| ${separator.join(" | ")} |`,
		...body.map((row) => `| ${row.join(" | ")} |`),
	].join("\n");
}

async function downloadCSV(data: string[][], filename = "table.csv") {
	const csv = tableToCSV(data);

	// Check if we're in Tauri environment
	if (typeof window !== "undefined" && "__TAURI__" in window) {
		try {
			const { save } = await import("@tauri-apps/plugin-dialog");
			const { writeTextFile } = await import("@tauri-apps/plugin-fs");

			const filePath = await save({
				canCreateDirectories: true,
				title: "Save CSV",
				defaultPath: filename,
				filters: [{ name: "CSV", extensions: ["csv"] }],
			});

			if (filePath) {
				await writeTextFile(filePath, csv);
				return;
			}
			// User cancelled, don't fall through
			return;
		} catch (e) {
			console.warn("Tauri save failed, using fallback:", e);
			// Fall through to data URL download
		}
	}

	// Universal fallback using data URL (works in both browser and Tauri webview)
	const dataUrl = `data:text/csv;charset=utf-8,${encodeURIComponent(csv)}`;
	const link = document.createElement("a");
	link.href = dataUrl;
	link.download = filename;
	link.style.display = "none";
	document.body.appendChild(link);
	link.click();
	document.body.removeChild(link);
}

// Renders cell content with support for markdown images and links
function CellContent({ content }: { content: string }) {
	const parts = React.useMemo(() => {
		const result: React.ReactNode[] = [];
		let lastIndex = 0;
		let key = 0;

		// Combined regex to match both images and links
		const combinedRegex = /(!?\[([^\]]*)\]\(([^)]+)\))/g;
		let match: RegExpExecArray | null;

		while ((match = combinedRegex.exec(content)) !== null) {
			// Add text before the match
			if (match.index > lastIndex) {
				result.push(
					<span key={key++}>{content.slice(lastIndex, match.index)}</span>,
				);
			}

			const fullMatch = match[1];
			const isImage = fullMatch.startsWith("!");
			const altOrText = match[2];
			const url = match[3];

			if (isImage) {
				const sanitizedUrl = sanitizeImageUrl(url, "");
				if (sanitizedUrl) {
					result.push(
						<img
							key={key++}
							src={sanitizedUrl}
							alt={altOrText}
							className="max-h-24 max-w-[200px] rounded object-contain inline-block align-middle"
							loading="lazy"
						/>,
					);
				} else {
					// If URL is not safe, show alt text
					result.push(<span key={key++}>[{altOrText || "image"}]</span>);
				}
			} else {
				result.push(
					<a
						key={key++}
						href={url}
						target="_blank"
						rel="noopener noreferrer"
						className="text-primary hover:underline"
					>
						{altOrText}
					</a>,
				);
			}

			lastIndex = match.index + fullMatch.length;
		}

		// Add remaining text
		if (lastIndex < content.length) {
			result.push(<span key={key++}>{content.slice(lastIndex)}</span>);
		}

		return result.length > 0 ? result : content;
	}, [content]);

	return <>{parts}</>;
}

// Sortable column item for the popover list
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
				title={header || `Column ${colIdx + 1}`}
			>
				{header || `Column ${colIdx + 1}`}
			</label>
			{isHidden ? (
				<EyeOffIcon className="h-3 w-3 text-muted-foreground shrink-0" />
			) : (
				<EyeIcon className="h-3 w-3 text-muted-foreground shrink-0" />
			)}
		</div>
	);
}

// Sortable table header cell
function SortableHeaderCell({
	colIdx,
	header,
	sortColumn,
	sortDirection,
	onSort,
}: {
	colIdx: number;
	header: string;
	sortColumn: number | null;
	sortDirection: SortDirection;
	onSort: (colIdx: number) => void;
}) {
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
		<th
			ref={setNodeRef}
			style={style}
			className={cn(
				"text-left font-semibold px-2 py-1.5 hover:bg-muted/50 transition-colors whitespace-nowrap select-none",
				isDragging && "opacity-50 bg-muted",
			)}
		>
			<div className="flex items-center gap-1">
				<GripVerticalIcon
					className="h-3 w-3 text-muted-foreground/50 shrink-0 cursor-grab"
					{...attributes}
					{...listeners}
				/>
				<span
					className="truncate max-w-[200px] cursor-pointer"
					title={header || `Column ${colIdx + 1}`}
					onClick={() => onSort(colIdx)}
				>
					{header || `Column ${colIdx + 1}`}
				</span>
				{sortColumn === colIdx ? (
					sortDirection === "asc" ? (
						<ArrowUpIcon className="h-3 w-3 text-primary shrink-0" />
					) : sortDirection === "desc" ? (
						<ArrowDownIcon className="h-3 w-3 text-primary shrink-0" />
					) : (
						<ArrowUpDownIcon className="h-3 w-3 text-muted-foreground/50 shrink-0" />
					)
				) : (
					<ArrowUpDownIcon className="h-3 w-3 text-muted-foreground/50 opacity-0 group-hover/table:opacity-100 shrink-0" />
				)}
			</div>
		</th>
	);
}

export function TableViewer({ data, children, className }: TableViewerProps) {
	const [copied, setCopied] = React.useState(false);
	const [searchQuery, setSearchQuery] = React.useState("");
	const [sortColumn, setSortColumn] = React.useState<number | null>(null);
	const [sortDirection, setSortDirection] = React.useState<SortDirection>(null);
	const [columnFilters, setColumnFilters] = React.useState<ColumnFilter[]>([]);
	const [showFilters, setShowFilters] = React.useState(false);
	const [showColumns, setShowColumns] = React.useState(false);
	const [isExpanded, setIsExpanded] = React.useState(false);
	const [wrapCells, setWrapCells] = React.useState(false);
	const [expandedCells, setExpandedCells] = React.useState<Set<string>>(
		() => new Set(),
	);

	const hasHeader = data.length > 0;
	const headers = hasHeader ? data[0] : [];
	const rows = hasHeader ? data.slice(1) : [];

	// Column order and visibility state
	const [columnOrder, setColumnOrder] = React.useState<number[]>(() =>
		headers.map((_, i) => i),
	);
	const [hiddenColumns, setHiddenColumns] = React.useState<Set<number>>(
		() => new Set(),
	);

	// dnd-kit sensors
	const sensors = useSensors(
		useSensor(PointerSensor, {
			activationConstraint: {
				distance: 5,
			},
		}),
	);

	// Reset column order when headers change
	React.useEffect(() => {
		setColumnOrder(headers.map((_, i) => i));
		setHiddenColumns(new Set());
		setExpandedCells(new Set());
	}, [headers.length]);

	// Visible columns in order
	const visibleColumns = React.useMemo(
		() => columnOrder.filter((idx) => !hiddenColumns.has(idx)),
		[columnOrder, hiddenColumns],
	);

	// Column IDs for dnd-kit (needs string IDs)
	const columnIds = React.useMemo(
		() => columnOrder.map((idx) => `col-${idx}`),
		[columnOrder],
	);

	const visibleColumnIds = React.useMemo(
		() => visibleColumns.map((idx) => `col-${idx}`),
		[visibleColumns],
	);

	// Apply search filter
	const searchedRows = React.useMemo(() => {
		if (!searchQuery.trim()) return rows;
		const query = searchQuery.toLowerCase();
		return rows.filter((row) =>
			row.some((cell) => cell.toLowerCase().includes(query)),
		);
	}, [rows, searchQuery]);

	// Apply column filters
	const filteredRows = React.useMemo(() => {
		if (columnFilters.length === 0) return searchedRows;
		return searchedRows.filter((row) =>
			columnFilters.every((filter) => {
				const cellValue = row[filter.column]?.toLowerCase() || "";
				return cellValue.includes(filter.value.toLowerCase());
			}),
		);
	}, [searchedRows, columnFilters]);

	// Apply sorting
	const sortedRows = React.useMemo(() => {
		if (sortColumn === null || sortDirection === null) return filteredRows;
		return [...filteredRows].sort((a, b) => {
			const aVal = a[sortColumn] || "";
			const bVal = b[sortColumn] || "";

			// Try numeric comparison first
			const aNum = Number.parseFloat(aVal);
			const bNum = Number.parseFloat(bVal);
			if (!isNaN(aNum) && !isNaN(bNum)) {
				return sortDirection === "asc" ? aNum - bNum : bNum - aNum;
			}

			// Fall back to string comparison
			const comparison = aVal.localeCompare(bVal);
			return sortDirection === "asc" ? comparison : -comparison;
		});
	}, [filteredRows, sortColumn, sortDirection]);

	const handleCopy = React.useCallback(
		(format: "csv" | "markdown") => {
			// Only export visible columns in order
			const visibleHeaders = visibleColumns.map((idx) => headers[idx]);
			const visibleRows = sortedRows.map((row) =>
				visibleColumns.map((idx) => row[idx]),
			);
			const exportData = [visibleHeaders, ...visibleRows];
			const text =
				format === "csv" ? tableToCSV(exportData) : tableToMarkdown(exportData);
			navigator.clipboard.writeText(text);
			setCopied(true);
			setTimeout(() => setCopied(false), 2000);
		},
		[headers, sortedRows, visibleColumns],
	);

	const handleDownload = React.useCallback(() => {
		// Only export visible columns in order
		const visibleHeaders = visibleColumns.map((idx) => headers[idx]);
		const visibleRows = sortedRows.map((row) =>
			visibleColumns.map((idx) => row[idx]),
		);
		const exportData = [visibleHeaders, ...visibleRows];
		downloadCSV(exportData);
	}, [headers, sortedRows, visibleColumns]);

	const handleColumnSort = React.useCallback((columnIndex: number) => {
		setSortColumn((prevCol) => {
			if (prevCol !== columnIndex) {
				setSortDirection("asc");
				return columnIndex;
			}
			// Toggle through: asc -> desc -> none
			setSortDirection((prevDir) => {
				if (prevDir === "asc") return "desc";
				if (prevDir === "desc") return null;
				return "asc";
			});
			return columnIndex;
		});
	}, []);

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

	// Keyed by content, not row index, so expansion survives sorting/filtering
	const toggleCellExpanded = React.useCallback((cellKey: string) => {
		setExpandedCells((prev) => {
			const next = new Set(prev);
			if (next.has(cellKey)) {
				next.delete(cellKey);
			} else {
				next.add(cellKey);
			}
			return next;
		});
	}, []);

	const toggleColumnVisibility = React.useCallback(
		(columnIndex: number) => {
			setHiddenColumns((prev) => {
				const next = new Set(prev);
				if (next.has(columnIndex)) {
					next.delete(columnIndex);
				} else {
					// Don't hide if it's the last visible column
					if (next.size >= headers.length - 1) return prev;
					next.add(columnIndex);
				}
				return next;
			});
		},
		[headers.length],
	);

	// Handle drag end for column reordering (popover list)
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

	// Handle drag end for header column reordering
	const handleHeaderDragEnd = React.useCallback((event: DragEndEvent) => {
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
		setColumnOrder(headers.map((_, i) => i));
		setHiddenColumns(new Set());
	}, [headers]);

	const clearFilters = React.useCallback(() => {
		setSearchQuery("");
		setColumnFilters([]);
		setSortColumn(null);
		setSortDirection(null);
	}, []);

	const hasActiveFilters =
		searchQuery.trim() || columnFilters.length > 0 || sortColumn !== null;

	const hasColumnChanges =
		hiddenColumns.size > 0 || !columnOrder.every((col, idx) => col === idx);

	const resultCount = sortedRows.length;
	const totalCount = rows.length;

	return (
		<div className={cn("group/table relative", className)}>
			{/* Toolbar */}
			<div className="flex items-center gap-1 mb-2 flex-wrap opacity-0 transition-opacity group-hover/table:opacity-100 focus-within:opacity-100">
				{/* Search */}
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

				{/* Column Filters */}
				<Popover open={showFilters} onOpenChange={setShowFilters}>
					<PopoverTrigger asChild>
						<Button
							variant={columnFilters.length > 0 ? "secondary" : "ghost"}
							size="sm"
							className="h-7 px-2 text-xs"
						>
							<FilterIcon className="h-3 w-3 mr-1" />
							Filter
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
								Filter by column
							</div>
							{headers.map((header, idx) => (
								<div key={idx} className="flex items-center gap-2">
									<span className="text-xs w-20 truncate" title={header}>
										{header || `Col ${idx + 1}`}
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

				{/* Column visibility and order */}
				<Popover open={showColumns} onOpenChange={setShowColumns}>
					<PopoverTrigger asChild>
						<Button
							variant={hasColumnChanges ? "secondary" : "ghost"}
							size="sm"
							className="h-7 px-2 text-xs"
						>
							<ColumnsIcon className="h-3 w-3 mr-1" />
							Columns
							{hiddenColumns.size > 0 && (
								<span className="ml-1 bg-muted-foreground text-muted rounded-full px-1.5 text-[10px]">
									{headers.length - hiddenColumns.size}/{headers.length}
								</span>
							)}
						</Button>
					</PopoverTrigger>
					<PopoverContent className="w-56 p-3" align="start">
						<div className="space-y-1">
							<div className="flex items-center justify-between mb-2">
								<span className="text-xs font-medium text-muted-foreground">
									Show/hide & reorder
								</span>
								{hasColumnChanges && (
									<Button
										variant="ghost"
										size="sm"
										className="h-5 px-1 text-[10px]"
										onClick={resetColumns}
									>
										Reset
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
											onToggleVisibility={() => toggleColumnVisibility(colIdx)}
										/>
									))}
								</SortableContext>
							</DndContext>
						</div>
					</PopoverContent>
				</Popover>

				{/* Wrap every cell instead of truncating */}
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant={wrapCells ? "secondary" : "ghost"}
							size="sm"
							className="h-7 px-2 text-xs"
							onClick={() => setWrapCells((prev) => !prev)}
						>
							<WrapTextIcon className="h-3 w-3 mr-1" />
							Wrap
						</Button>
					</TooltipTrigger>
					<TooltipContent side="top">
						{wrapCells ? "Truncate long cells" : "Show full text in every cell"}
					</TooltipContent>
				</Tooltip>

				{/* Clear filters */}
				{hasActiveFilters && (
					<Button
						variant="ghost"
						size="sm"
						className="h-7 px-2 text-xs"
						onClick={clearFilters}
					>
						<XIcon className="h-3 w-3 mr-1" />
						Clear
					</Button>
				)}

				<div className="flex-1" />

				{/* Result count */}
				{hasActiveFilters && (
					<span className="text-xs text-muted-foreground">
						{resultCount} of {totalCount} rows
					</span>
				)}

				{/* Copy */}
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7"
							onClick={() => handleCopy("markdown")}
						>
							{copied ? (
								<CheckIcon className="h-3.5 w-3.5 text-green-500" />
							) : (
								<CopyIcon className="h-3.5 w-3.5" />
							)}
						</Button>
					</TooltipTrigger>
					<TooltipContent side="top">Copy as Markdown</TooltipContent>
				</Tooltip>

				{/* Download */}
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7"
							onClick={handleDownload}
						>
							<DownloadIcon className="h-3.5 w-3.5" />
						</Button>
					</TooltipTrigger>
					<TooltipContent side="top">Download CSV</TooltipContent>
				</Tooltip>
			</div>

			{/* Table container with scroll and max height */}
			<div
				className={cn(
					"overflow-auto border border-border rounded-md",
					isExpanded ? "max-h-[70vh]" : "max-h-[400px]",
				)}
			>
				<DndContext
					sensors={sensors}
					collisionDetection={closestCenter}
					onDragEnd={handleHeaderDragEnd}
				>
					<table className="w-full text-sm border-collapse">
						{/* Sticky header */}
						{hasHeader && (
							<thead className="sticky top-0 z-10 bg-muted/80 backdrop-blur-sm border-b border-border">
								<SortableContext
									items={visibleColumnIds}
									strategy={horizontalListSortingStrategy}
								>
									<tr>
										{visibleColumns.map((colIdx) => (
											<SortableHeaderCell
												key={colIdx}
												colIdx={colIdx}
												header={headers[colIdx]}
												sortColumn={sortColumn}
												sortDirection={sortDirection}
												onSort={handleColumnSort}
											/>
										))}
									</tr>
								</SortableContext>
							</thead>
						)}
						<tbody>
							{sortedRows.length === 0 ? (
								<tr>
									<td
										colSpan={visibleColumns.length || 1}
										className="text-center py-8 text-muted-foreground text-sm"
									>
										{hasActiveFilters ? "No matching rows" : "No data"}
									</td>
								</tr>
							) : (
								sortedRows.map((row, rowIdx) => (
									<tr
										key={rowIdx}
										className={cn(
											"border-b border-border/50 last:border-0",
											rowIdx % 2 === 1 && "bg-muted/30",
										)}
									>
										{visibleColumns.map((colIdx) => {
											const cell = row[colIdx] || "";
											const hasMarkdownContent =
												cell.includes("![") || cell.includes("](");
											const isLongText =
												cell.length > 100 && !hasMarkdownContent;
											const cellKey = `${colIdx} ${cell}`;
											// Cells that already show everything need no toggle
											const isToggleable =
												!hasMarkdownContent && !isLongText && !wrapCells;
											const showsFullText =
												!isToggleable || expandedCells.has(cellKey);
											return (
												<td
													key={colIdx}
													className={cn(
														"px-2 py-1.5 align-top",
														showsFullText && !hasMarkdownContent
															? "min-w-[200px] max-w-[400px]"
															: "max-w-[300px]",
													)}
												>
													{hasMarkdownContent ? (
														<div className="flex flex-wrap items-center gap-1">
															<CellContent content={cell} />
														</div>
													) : isToggleable ? (
														<button
															type="button"
															tabIndex={-1}
															onClick={() => toggleCellExpanded(cellKey)}
															title={
																showsFullText
																	? "Click to collapse"
																	: `${cell}\n\nClick to expand`
															}
															className={cn(
																"block w-full cursor-pointer text-left",
																showsFullText
																	? "whitespace-pre-wrap wrap-break-word text-sm leading-relaxed"
																	: "truncate",
															)}
														>
															{cell}
														</button>
													) : (
														<div className="whitespace-pre-wrap wrap-break-word text-sm leading-relaxed">
															{cell}
														</div>
													)}
												</td>
											);
										})}
									</tr>
								))
							)}
						</tbody>
					</table>
				</DndContext>
			</div>

			{/* Expand/collapse for large tables */}
			{rows.length > 10 && (
				<button
					type="button"
					onClick={() => setIsExpanded(!isExpanded)}
					className="w-full mt-1 py-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
				>
					{isExpanded ? "Show less" : `Show more (${rows.length} rows)`}
				</button>
			)}

			{/* Hidden children for Slate to track elements. Inline style, not a
			    utility class: descendant rules like `[&_table]:block` outrank
			    `.hidden` and would render this tracking table as a duplicate. */}
			<table style={{ display: "none" }} aria-hidden="true">
				<tbody>{children}</tbody>
			</table>
		</div>
	);
}

// Re-export utilities for use in table components
export { tableToCSV, tableToMarkdown, downloadCSV };
