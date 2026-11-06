"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type ColumnDef,
	type SortingState,
	getCoreRowModel,
	getSortedRowModel,
	useReactTable,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
	ArrowDown,
	ArrowUp,
	Binary,
	Braces,
	Calendar,
	ChevronsUpDown,
	Copy,
	Expand,
	FileIcon,
	Hash,
	MapPinIcon,
	MoreHorizontal,
	ToggleLeft,
	Type,
	UserRound,
} from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { cn } from "../../../../lib/utils";
import type { QueryColumn } from "../../../../state/backend-state/query-state";
import { Button } from "../../../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import {
	CellInspectorDialog,
	type InspectedResultCell,
} from "./cell-inspector-dialog";
import {
	type ColumnKind,
	cellToString,
	classifyResultColumn,
	isNullish,
} from "./column-types";
import { ResultCellValue } from "./result-value";
import { RowInspectorSheet } from "./row-inspector-sheet";

const GUTTER_WIDTH = 52;
const ROW_HEIGHT = 33;

type ResultRow = Record<string, unknown>;

interface ColumnMeta {
	kind: ColumnKind;
	typeName: string;
	metadata?: Record<string, string>;
}

const KIND_ICON: Record<ColumnKind, typeof Hash> = {
	geometry: MapPinIcon,
	binary: Binary,
	number: Hash,
	temporal: Calendar,
	boolean: ToggleLeft,
	json: Braces,
	user: UserRound,
	file: FileIcon,
	text: Type,
};

function sizeForKind(kind: ColumnKind): number {
	if (kind === "number" || kind === "boolean") return 130;
	if (kind === "temporal" || kind === "user") return 190;
	if (kind === "file" || kind === "geometry" || kind === "binary") return 240;
	return 200;
}

function copyText(value: string, label: string): void {
	void navigator.clipboard
		.writeText(value)
		.then(() => toast.success(label))
		.catch(() => toast.error("Could not copy to clipboard"));
}

function HeaderCell({
	name,
	label = name,
	kind,
	typeName,
	sorted,
	onSort,
	onCopyColumn,
	canResize,
	onResizeStart,
	isResizing,
}: Readonly<{
	name: string;
	label?: string;
	kind: ColumnKind;
	typeName: string;
	sorted: false | "asc" | "desc";
	onSort: (event: React.MouseEvent) => void;
	onCopyColumn: () => void;
	canResize: boolean;
	onResizeStart: (event: React.MouseEvent | React.TouchEvent) => void;
	isResizing: boolean;
}>) {
	const { t } = useTranslation("settings");
	const Icon = KIND_ICON[kind];
	const SortIcon =
		sorted === "asc" ? ArrowUp : sorted === "desc" ? ArrowDown : ChevronsUpDown;
	return (
		<div className="group/head relative flex h-full w-full items-center gap-1.5 px-3">
			<Icon className="h-3 w-3 shrink-0 text-muted-foreground/70" aria-hidden />
			<button
				type="button"
				onClick={onSort}
				className="flex min-w-0 flex-1 items-center gap-1 rounded-sm text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				title={`${name} · ${typeName || "unknown"}`}
			>
				<span
					className={cn(
						"truncate text-xs font-medium text-foreground",
						label === name && "font-mono",
					)}
				>
					{label}
				</span>
				<SortIcon
					className={cn(
						"h-3 w-3 shrink-0 transition-opacity",
						sorted
							? "text-foreground opacity-100"
							: "text-muted-foreground opacity-0 group-hover/head:opacity-60",
					)}
					aria-hidden
				/>
			</button>
			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<Button
						variant="ghost"
						size="icon"
						className="h-6 w-6 shrink-0 text-muted-foreground opacity-0 focus-visible:opacity-100 group-hover/head:opacity-100"
						aria-label={t("nameColumnActions", "{{name}} column actions", {
							name,
						})}
					>
						<MoreHorizontal className="h-3.5 w-3.5" />
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="end" className="w-44">
					<DropdownMenuLabel className="truncate font-mono text-xs">
						{name}
					</DropdownMenuLabel>
					<DropdownMenuSeparator />
					<DropdownMenuItem onClick={onCopyColumn}>
						<Copy className="h-3.5 w-3.5" /> {t("copyColumn", "Copy column")}
					</DropdownMenuItem>
				</DropdownMenuContent>
			</DropdownMenu>
			{canResize && (
				<button
					type="button"
					tabIndex={-1}
					aria-label={t("resizeNameColumn", "Resize {{name}} column", { name })}
					onMouseDown={onResizeStart}
					onTouchStart={onResizeStart}
					className={cn(
						"absolute right-0 top-0 h-full w-1 cursor-col-resize touch-none select-none bg-transparent transition-colors hover:bg-primary/60",
						isResizing && "bg-primary",
					)}
				/>
			)}
		</div>
	);
}

/** How a host that knows more about its columns than their names wants them shown. */
export interface ResultTablePresentation {
	/** Header text; the raw column name stays in the tooltip and menus. */
	label?: (name: string, kind: ColumnKind) => string;
	/** The kind to show a column as, when the host knows better than the column alone. */
	kind?: (column: QueryColumn) => ColumnKind | undefined;
	/** A cell reading that replaces the default one; undefined keeps the default. */
	cell?: (name: string, value: unknown) => React.ReactNode | undefined;
}

export function QueryResultTable({
	columns,
	rows,
	appId,
	presentation,
}: Readonly<{
	columns: QueryColumn[];
	rows: ResultRow[];
	appId?: string;
	presentation?: ResultTablePresentation;
}>) {
	const { t } = useTranslation("settings");
	const scrollRef = useRef<HTMLDivElement>(null);
	const [sorting, setSorting] = useState<SortingState>([]);
	const [inspect, setInspect] = useState<ResultRow | null>(null);
	const [inspectedCell, setInspectedCell] =
		useState<InspectedResultCell | null>(null);

	const metaById = useMemo(() => {
		const map = new Map<string, ColumnMeta>();
		for (const column of columns)
			map.set(column.name, {
				kind:
					presentation?.kind?.(column) ??
					classifyResultColumn(column, rows, appId),
				typeName: column.type_name,
				metadata: column.metadata,
			});
		return map;
	}, [columns, rows, appId, presentation]);

	const columnDefs = useMemo<ColumnDef<ResultRow>[]>(
		() =>
			columns.map((column) => {
				const kind = metaById.get(column.name)?.kind ?? "text";
				return {
					id: column.name,
					// Map SQL NULL (JS null) to undefined so `sortUndefined: "last"`
					// orders nulls consistently last in both directions; isNullish still
					// renders them as NULL.
					accessorFn: (row) => {
						const value = row[column.name];
						return isNullish(value) ? undefined : value;
					},
					size: sizeForKind(kind),
					minSize: 72,
					maxSize: 640,
					sortUndefined: "last",
					enableSorting: kind !== "binary",
					sortingFn:
						kind === "number"
							? (a, b, id) => {
									const av = Number(a.getValue(id));
									const bv = Number(b.getValue(id));
									if (!Number.isFinite(av)) return Number.isFinite(bv) ? -1 : 0;
									if (!Number.isFinite(bv)) return 1;
									return av - bv;
								}
							: "alphanumeric",
				};
			}),
		[columns, metaById],
	);

	const table = useReactTable({
		data: rows,
		columns: columnDefs,
		state: { sorting },
		onSortingChange: setSorting,
		getCoreRowModel: getCoreRowModel(),
		getSortedRowModel: getSortedRowModel(),
		enableColumnResizing: true,
		columnResizeMode: "onChange",
	});

	const rowModel = table.getRowModel();

	const rowVirtualizer = useVirtualizer({
		count: rowModel.rows.length,
		getScrollElement: () => scrollRef.current,
		estimateSize: () => ROW_HEIGHT,
		overscan: 14,
	});

	const totalWidth = GUTTER_WIDTH + table.getTotalSize();

	const copyColumn = (id: string) => {
		const values = rowModel.rows
			.map((row) => cellToString(row.getValue(id)))
			.join("\n");
		copyText(values, `Copied ${rowModel.rows.length} values`);
	};

	const headers = table.getHeaderGroups()[0]?.headers ?? [];
	const virtualRows = rowVirtualizer.getVirtualItems();
	const columnCount = columns.length + 1;
	const paddingTop = virtualRows.length > 0 ? virtualRows[0].start : 0;
	const paddingBottom =
		virtualRows.length > 0
			? rowVirtualizer.getTotalSize() - virtualRows[virtualRows.length - 1].end
			: 0;

	// Native <table> layout (no display override) keeps table semantics intact
	// across engines, incl. WebKit; rows are virtualized via spacer rows.
	return (
		<>
			<div ref={scrollRef} className="h-full overflow-auto">
				<table
					aria-rowcount={rowModel.rows.length + 1}
					className="m-0 table-fixed border-separate border-spacing-0 text-sm"
					style={{ width: totalWidth, minWidth: "100%" }}
				>
					<caption className="sr-only">
						{t("queryResults", "Query results")}
					</caption>
					<thead>
						<tr aria-rowindex={1}>
							<th
								scope="col"
								className="sticky left-0 top-0 z-30 h-9 border-b border-r border-l-0 border-t-0 bg-muted py-0 text-[10px] font-medium uppercase tracking-wider text-muted-foreground"
								style={{ width: GUTTER_WIDTH }}
							>
								#
							</th>
							{headers.map((header) => {
								const meta = metaById.get(header.column.id) ?? {
									kind: "text" as ColumnKind,
									typeName: "",
								};
								const sorted = header.column.getIsSorted();
								return (
									<th
										key={header.id}
										scope="col"
										aria-sort={
											sorted === "asc"
												? "ascending"
												: sorted === "desc"
													? "descending"
													: "none"
										}
										className="sticky top-0 z-20 h-9 border-b border-r border-l-0 border-t-0 bg-background p-0 font-normal"
										style={{ width: header.getSize() }}
									>
										<HeaderCell
											name={header.column.id}
											label={presentation?.label?.(header.column.id, meta.kind)}
											kind={meta.kind}
											typeName={meta.typeName}
											sorted={sorted}
											onSort={(event) => {
												if (!header.column.getCanSort()) return;
												header.column.toggleSorting(undefined, event.shiftKey);
											}}
											onCopyColumn={() => copyColumn(header.column.id)}
											canResize={header.column.getCanResize()}
											onResizeStart={header.getResizeHandler()}
											isResizing={header.column.getIsResizing()}
										/>
									</th>
								);
							})}
						</tr>
					</thead>

					<tbody>
						{paddingTop > 0 && (
							<tr aria-hidden>
								<td
									className="border-0 p-0"
									colSpan={columnCount}
									style={{ height: paddingTop }}
								/>
							</tr>
						)}
						{virtualRows.map((virtualRow) => {
							const row = rowModel.rows[virtualRow.index];
							return (
								<tr
									key={row.id}
									aria-rowindex={virtualRow.index + 2}
									className="group/row even:bg-muted/15 transition-colors hover:bg-muted/40"
									style={{ height: ROW_HEIGHT }}
								>
									<td
										className="sticky left-0 z-10 border-b border-r border-l-0 border-t-0 bg-muted p-0 group-hover/row:bg-muted"
										style={{ width: GUTTER_WIDTH }}
									>
										<button
											type="button"
											onClick={() => setInspect(row.original)}
											className="flex w-full items-center justify-end px-2 font-mono text-[11px] tabular-nums text-muted-foreground transition-colors hover:bg-muted-foreground/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
											style={{ height: ROW_HEIGHT - 1 }}
											aria-label={t("inspectRowVal", "Inspect row {{val}}", {
												val: virtualRow.index + 1,
											})}
										>
											{virtualRow.index + 1}
										</button>
									</td>
									{row.getVisibleCells().map((cell) => {
										const meta = metaById.get(cell.column.id) ?? {
											kind: "text" as ColumnKind,
											typeName: "",
										};
										const value = cell.getValue();
										const inspectCell = () => {
											const column = columns.find(
												(column) => column.name === cell.column.id,
											);
											if (column)
												setInspectedCell({ column, kind: meta.kind, value });
										};
										return (
											<td
												key={cell.id}
												className={cn(
													"group/cell relative overflow-hidden border-b border-r border-l-0 border-t-0 px-3 py-0 font-mono text-xs",
													meta.kind === "text" &&
														!/(^id$|_id$|^uuid$|^key$)/i.test(cell.column.id) &&
														"font-sans text-[13px]",
													meta.kind === "number" && "text-right",
													meta.kind === "temporal" && "text-muted-foreground",
												)}
												style={{ width: cell.column.getSize() }}
											>
												<div
													className={cn(
														"flex items-center overflow-hidden",
														meta.kind === "number" && "justify-end",
													)}
													style={{ height: ROW_HEIGHT - 1 }}
												>
													{presentation?.cell?.(cell.column.id, value) ?? (
														<ResultCellValue
															value={value}
															kind={meta.kind}
															metadata={meta.metadata}
															name={cell.column.id}
															appId={appId}
															onInspect={inspectCell}
														/>
													)}
												</div>
												{!isNullish(value) && (
													<div className="absolute right-1 top-1/2 flex -translate-y-1/2 items-center rounded bg-background/95 opacity-0 transition-opacity focus-within:opacity-100 group-hover/cell:opacity-100">
														<button
															type="button"
															onClick={inspectCell}
															className="flex h-6 w-6 items-center justify-center rounded text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
															aria-label={t("expandName", "Expand {{name}}", {
																name: cell.column.id,
															})}
														>
															<Expand className="h-3 w-3" />
														</button>
														<button
															type="button"
															onClick={() =>
																copyText(cellToString(value), "Copied cell")
															}
															className="flex h-6 w-6 items-center justify-center rounded text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
															aria-label={t("copyCellValue", "Copy cell value")}
														>
															<Copy className="h-3 w-3" />
														</button>
													</div>
												)}
											</td>
										);
									})}
								</tr>
							);
						})}
						{paddingBottom > 0 && (
							<tr aria-hidden>
								<td
									className="border-0 p-0"
									colSpan={columnCount}
									style={{ height: paddingBottom }}
								/>
							</tr>
						)}
					</tbody>
				</table>
			</div>
			<CellInspectorDialog
				cell={inspectedCell}
				appId={appId}
				onOpenChange={(open) => {
					if (!open) setInspectedCell(null);
				}}
			/>
			<RowInspectorSheet
				row={inspect}
				columns={columns}
				appId={appId}
				kindOf={(column) => metaById.get(column.name)?.kind ?? "text"}
				onOpenChange={(open) => {
					if (!open) setInspect(null);
				}}
			/>
		</>
	);
}
