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
	Braces,
	Calendar,
	ChevronsUpDown,
	Copy,
	FileIcon,
	Hash,
	MoreHorizontal,
	ToggleLeft,
	Type,
	UserRound,
} from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { resolveStorageFile } from "../../../../lib/storage-file";
import { cn } from "../../../../lib/utils";
import type { QueryColumn } from "../../../../state/backend-state/query-state";
import { accountIdFromValue } from "../../../../state/backend-state/user-state";
import { Button } from "../../../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { RelativeTime } from "../../../ui/relative-time";
import { StorageFileCell } from "../../../ui/storage-file-cell";
import { UserInlineTag } from "../../../ui/user-identity";
import {
	type ColumnKind,
	cellToString,
	classifyResultColumn,
	formatNumber,
	isNullish,
} from "./column-types";
import { RowInspectorSheet } from "./row-inspector-sheet";

const GUTTER_WIDTH = 52;
const ROW_HEIGHT = 33;

type ResultRow = Record<string, unknown>;

interface ColumnMeta {
	kind: ColumnKind;
	typeName: string;
}

const KIND_ICON: Record<ColumnKind, typeof Hash> = {
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
	if (kind === "file") return 240;
	return 200;
}

function copyText(value: string, label: string): void {
	void navigator.clipboard.writeText(value);
	toast.success(label);
}

function CellContent({
	value,
	kind,
	name,
	appId,
}: Readonly<{
	value: unknown;
	kind: ColumnKind;
	name: string;
	appId?: string;
}>) {
	const { t } = useTranslation("settings");
	if (isNullish(value)) {
		return (
			<span className="select-none italic text-muted-foreground/50">{`NULL`}</span>
		);
	}
	if (kind === "boolean") {
		const truthy = value === true || value === "true" || value === 1;
		return (
			<span className="flex items-center gap-1.5">
				<span
					className={cn(
						"h-1.5 w-1.5 rounded-full",
						truthy ? "bg-chart-2" : "bg-muted-foreground/40",
					)}
				/>
				{String(value)}
			</span>
		);
	}
	if (kind === "number") {
		return <span className="tabular-nums">{formatNumber(value)}</span>;
	}
	if (kind === "temporal") {
		return <RelativeTime value={value} className="min-w-0 truncate" />;
	}
	// A user column still holds text for rows that name no account, so the tag is
	// used only where the value is an id the directory could answer for.
	if (kind === "user") {
		const userId = accountIdFromValue(value);
		if (userId) return <UserInlineTag userId={userId} />;
	}
	// Same story for files: the column holds paths, but a row may hold a path that
	// points nowhere this app can open, and that row stays text.
	if (kind === "file") {
		const file = resolveStorageFile(name, value, appId);
		if (file && appId) return <StorageFileCell appId={appId} file={file} />;
	}
	return <span className="min-w-0 truncate">{cellToString(value)}</span>;
}

function HeaderCell({
	name,
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
				<span className="truncate font-mono text-xs font-medium text-foreground">
					{name}
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

export function QueryResultTable({
	columns,
	rows,
	appId,
}: Readonly<{
	columns: QueryColumn[];
	rows: ResultRow[];
	appId?: string;
}>) {
	const { t } = useTranslation("settings");
	const scrollRef = useRef<HTMLDivElement>(null);
	const [sorting, setSorting] = useState<SortingState>([]);
	const [inspect, setInspect] = useState<ResultRow | null>(null);

	const metaById = useMemo(() => {
		const map = new Map<string, ColumnMeta>();
		for (const column of columns)
			map.set(column.name, {
				kind: classifyResultColumn(column, rows, appId),
				typeName: column.type_name,
			});
		return map;
	}, [columns, rows, appId]);

	const columnDefs = useMemo<ColumnDef<ResultRow>[]>(
		() =>
			columns.map((column) => {
				const kind = classifyResultColumn(column, rows, appId);
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
		[columns, rows, appId],
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
					className="table-fixed border-separate border-spacing-0 text-sm"
					style={{ width: totalWidth, minWidth: "100%" }}
				>
					<caption className="sr-only">
						{t("queryResults", "Query results")}
					</caption>
					<thead>
						<tr aria-rowindex={1}>
							<th
								scope="col"
								className="sticky left-0 top-0 z-30 h-9 border-b border-r bg-muted text-[10px] font-medium uppercase tracking-wider text-muted-foreground"
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
										className="sticky top-0 z-20 h-9 border-b border-r bg-background p-0 font-normal"
										style={{ width: header.getSize() }}
									>
										<HeaderCell
											name={header.column.id}
											kind={meta.kind}
											typeName={meta.typeName}
											sorted={sorted}
											onSort={(event) =>
												header.column.toggleSorting(undefined, event.shiftKey)
											}
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
								<td colSpan={columnCount} style={{ height: paddingTop }} />
							</tr>
						)}
						{virtualRows.map((virtualRow) => {
							const row = rowModel.rows[virtualRow.index];
							return (
								<tr
									key={row.id}
									aria-rowindex={virtualRow.index + 2}
									className="group/row transition-colors hover:bg-muted/40"
									style={{ height: ROW_HEIGHT }}
								>
									<td
										className="sticky left-0 z-10 border-b border-r bg-muted p-0 group-hover/row:bg-muted"
										style={{ width: GUTTER_WIDTH }}
									>
										<button
											type="button"
											onClick={() => setInspect(row.original)}
											className="flex h-full w-full items-center justify-end px-2 font-mono text-[11px] tabular-nums text-muted-foreground transition-colors hover:bg-muted-foreground/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
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
										return (
											<td
												key={cell.id}
												className={cn(
													"group/cell relative overflow-hidden border-b border-r px-3 font-mono text-xs",
													meta.kind === "number" && "text-right",
													meta.kind === "temporal" && "text-muted-foreground",
												)}
												style={{ width: cell.column.getSize() }}
											>
												<div
													className={cn(
														"flex items-center",
														meta.kind === "number" && "justify-end",
													)}
												>
													<CellContent
														value={value}
														kind={meta.kind}
														name={cell.column.id}
														appId={appId}
													/>
												</div>
												{!isNullish(value) && (
													<button
														type="button"
														onClick={() =>
															copyText(cellToString(value), "Copied cell")
														}
														className="absolute right-1 top-1/2 flex h-5 w-5 -translate-y-1/2 items-center justify-center rounded bg-background/80 text-muted-foreground opacity-0 backdrop-blur-sm transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring group-hover/cell:opacity-100"
														aria-label={t("copyCellValue", "Copy cell value")}
													>
														<Copy className="h-3 w-3" />
													</button>
												)}
											</td>
										);
									})}
								</tr>
							);
						})}
						{paddingBottom > 0 && (
							<tr aria-hidden>
								<td colSpan={columnCount} style={{ height: paddingBottom }} />
							</tr>
						)}
					</tbody>
				</table>
			</div>
			<RowInspectorSheet
				row={inspect}
				columns={columns}
				appId={appId}
				onOpenChange={(open) => {
					if (!open) setInspect(null);
				}}
			/>
		</>
	);
}
