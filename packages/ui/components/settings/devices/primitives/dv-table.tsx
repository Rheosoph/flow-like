"use client";

import { ChevronDown, ChevronUp, ChevronsUpDown } from "lucide-react";
import type { ComponentProps, ReactNode } from "react";
import { cx } from "./tone";

/** Container width (px) below which rows become cards (SPEC §4.18). `false` never stacks. */
export type StackAt = 900 | 560 | false;

/*
 * Stacked cards are container-query driven on the table's own wrapper
 * (`@container/tbl`), so a table in a narrow sheet stacks even on a wide screen.
 * The class lists are literal so Tailwind sees every variant.
 */
const STACK: Record<Exclude<StackAt, false>, string> = {
	900: "@max-[900px]/tbl:block @max-[900px]/tbl:[&_colgroup]:hidden @max-[900px]/tbl:[&_thead]:hidden @max-[900px]/tbl:[&_tbody]:flex @max-[900px]/tbl:[&_tbody]:flex-col @max-[900px]/tbl:[&_tbody]:gap-2 @max-[900px]/tbl:[&_tbody]:p-3 @max-[900px]/tbl:[&_tr]:flex @max-[900px]/tbl:[&_tr]:flex-col @max-[900px]/tbl:[&_tr]:gap-2 @max-[900px]/tbl:[&_tr]:rounded-lg @max-[900px]/tbl:[&_tr]:border @max-[900px]/tbl:[&_tr]:border-border @max-[900px]/tbl:[&_tr]:bg-card @max-[900px]/tbl:[&_tr]:p-3 @max-[900px]/tbl:[&_tr:hover]:bg-card @max-[900px]/tbl:[&_td]:block @max-[900px]/tbl:[&_td]:border-0 @max-[900px]/tbl:[&_td]:p-0 @max-[900px]/tbl:[&_td:empty]:hidden @max-[900px]/tbl:[&_td[data-label]]:before:mb-0.5 @max-[900px]/tbl:[&_td[data-label]]:before:block @max-[900px]/tbl:[&_td[data-label]]:before:text-label @max-[900px]/tbl:[&_td[data-label]]:before:font-semibold @max-[900px]/tbl:[&_td[data-label]]:before:uppercase @max-[900px]/tbl:[&_td[data-label]]:before:tracking-[0.06em] @max-[900px]/tbl:[&_td[data-label]]:before:text-muted-foreground @max-[900px]/tbl:[&_td[data-label]]:before:content-[attr(data-label)] @max-[900px]/tbl:[&_tr>td:first-child]:before:content-none @max-[900px]/tbl:[&_tr>td[data-cell=act]]:border-t @max-[900px]/tbl:[&_tr>td[data-cell=act]]:border-hairline @max-[900px]/tbl:[&_tr>td[data-cell=act]]:pt-2 @max-[900px]/tbl:[&_tr>td[data-cell=act]]:text-left @max-[900px]/tbl:[&_tr>td[data-cell=act]]:before:content-none @max-[900px]/tbl:[&_tr>td[data-cell=more]]:border-t @max-[900px]/tbl:[&_tr>td[data-cell=more]]:border-hairline @max-[900px]/tbl:[&_tr>td[data-cell=more]]:pt-2 @max-[900px]/tbl:[&_tr>td[data-cell=more]]:text-left @max-[900px]/tbl:[&_tr>td[data-cell=more]]:before:content-none @max-[900px]/tbl:[&_td]:text-left @max-[900px]/tbl:[&_tr>td:first-child]:pl-0 @max-[900px]/tbl:[&_tr>td:last-child]:pr-0 @max-[900px]/tbl:[&_tr[aria-selected=true]]:bg-row-selected @max-[900px]/tbl:[&_tr[aria-selected=true]:hover]:bg-row-selected @max-[900px]/tbl:[&_tr[data-group]]:bg-surface-sunken @max-[900px]/tbl:[&_tr[data-group]]:px-3 @max-[900px]/tbl:[&_tr[data-group]]:py-2 @max-[900px]/tbl:[&_tr[data-group]>td]:bg-transparent @max-[900px]/tbl:[&_tr[data-confirm-row]]:border-0 @max-[900px]/tbl:[&_tr[data-confirm-row]]:bg-transparent @max-[900px]/tbl:[&_tr[data-confirm-row]]:p-0",
	560: "@max-[560px]/tbl:block @max-[560px]/tbl:[&_colgroup]:hidden @max-[560px]/tbl:[&_thead]:hidden @max-[560px]/tbl:[&_tbody]:flex @max-[560px]/tbl:[&_tbody]:flex-col @max-[560px]/tbl:[&_tbody]:gap-2 @max-[560px]/tbl:[&_tbody]:p-3 @max-[560px]/tbl:[&_tr]:flex @max-[560px]/tbl:[&_tr]:flex-col @max-[560px]/tbl:[&_tr]:gap-2 @max-[560px]/tbl:[&_tr]:rounded-lg @max-[560px]/tbl:[&_tr]:border @max-[560px]/tbl:[&_tr]:border-border @max-[560px]/tbl:[&_tr]:bg-card @max-[560px]/tbl:[&_tr]:p-3 @max-[560px]/tbl:[&_tr:hover]:bg-card @max-[560px]/tbl:[&_td]:block @max-[560px]/tbl:[&_td]:border-0 @max-[560px]/tbl:[&_td]:p-0 @max-[560px]/tbl:[&_td:empty]:hidden @max-[560px]/tbl:[&_td[data-label]]:before:mb-0.5 @max-[560px]/tbl:[&_td[data-label]]:before:block @max-[560px]/tbl:[&_td[data-label]]:before:text-label @max-[560px]/tbl:[&_td[data-label]]:before:font-semibold @max-[560px]/tbl:[&_td[data-label]]:before:uppercase @max-[560px]/tbl:[&_td[data-label]]:before:tracking-[0.06em] @max-[560px]/tbl:[&_td[data-label]]:before:text-muted-foreground @max-[560px]/tbl:[&_td[data-label]]:before:content-[attr(data-label)] @max-[560px]/tbl:[&_tr>td:first-child]:before:content-none @max-[560px]/tbl:[&_tr>td[data-cell=act]]:border-t @max-[560px]/tbl:[&_tr>td[data-cell=act]]:border-hairline @max-[560px]/tbl:[&_tr>td[data-cell=act]]:pt-2 @max-[560px]/tbl:[&_tr>td[data-cell=act]]:text-left @max-[560px]/tbl:[&_tr>td[data-cell=act]]:before:content-none @max-[560px]/tbl:[&_tr>td[data-cell=more]]:border-t @max-[560px]/tbl:[&_tr>td[data-cell=more]]:border-hairline @max-[560px]/tbl:[&_tr>td[data-cell=more]]:pt-2 @max-[560px]/tbl:[&_tr>td[data-cell=more]]:text-left @max-[560px]/tbl:[&_tr>td[data-cell=more]]:before:content-none @max-[560px]/tbl:[&_td]:text-left @max-[560px]/tbl:[&_tr>td:first-child]:pl-0 @max-[560px]/tbl:[&_tr>td:last-child]:pr-0 @max-[560px]/tbl:[&_tr[aria-selected=true]]:bg-row-selected @max-[560px]/tbl:[&_tr[aria-selected=true]:hover]:bg-row-selected @max-[560px]/tbl:[&_tr[data-group]]:bg-surface-sunken @max-[560px]/tbl:[&_tr[data-group]]:px-3 @max-[560px]/tbl:[&_tr[data-group]]:py-2 @max-[560px]/tbl:[&_tr[data-group]>td]:bg-transparent @max-[560px]/tbl:[&_tr[data-confirm-row]]:border-0 @max-[560px]/tbl:[&_tr[data-confirm-row]]:bg-transparent @max-[560px]/tbl:[&_tr[data-confirm-row]]:p-0",
};

export interface DvTableProps
	extends Omit<ComponentProps<"table">, "children"> {
	/** Column plan (SPEC §5): one width per column, e.g. `["18%", "auto", "120px"]`. */
	cols: readonly string[];
	/** The header row (`<tr>` of `Th`/`SortHeader`). */
	head?: ReactNode;
	stackAt?: StackAt;
	/** Accessible name of the table. */
	label: string;
	children?: ReactNode;
	wrapperClassName?: string;
}

/** SPEC §4.16: `table-fixed` with a column plan; the wrapper scrolls only as a safety net (R1). */
export function DvTable({
	cols,
	head,
	stackAt = 900,
	label,
	className,
	wrapperClassName,
	children,
	...props
}: DvTableProps) {
	return (
		<div
			data-slot="dv-table"
			className={cx("@container/tbl min-w-0 overflow-x-auto", wrapperClassName)}
		>
			<table
				aria-label={label}
				data-stack={stackAt || undefined}
				className={cx(
					"w-full table-fixed border-collapse text-ui",
					stackAt ? STACK[stackAt] : undefined,
					className,
				)}
				{...props}
			>
				<colgroup>
					{cols.map((width, index) => (
						<col
							// biome-ignore lint/suspicious/noArrayIndexKey: columns are positional by definition
							key={index}
							style={width === "auto" ? undefined : { width }}
						/>
					))}
				</colgroup>
				{head ? <thead>{head}</thead> : null}
				<tbody>{children}</tbody>
			</table>
		</div>
	);
}

const HEAD_CELL =
	"overflow-hidden border-b border-border bg-surface-sunken px-3 py-2 text-left align-bottom text-label font-semibold uppercase tracking-[0.06em] text-ellipsis whitespace-nowrap text-muted-foreground first:pl-4 last:pr-4";

export function Th({
	className,
	numeric = false,
	children,
	...props
}: ComponentProps<"th"> & { numeric?: boolean }) {
	return (
		<th
			scope="col"
			className={cx(HEAD_CELL, numeric && "text-right", className)}
			{...props}
		>
			{children}
		</th>
	);
}

export type SortDirection = "ascending" | "descending" | "none";

export function SortHeader({
	sort = "none",
	onSort,
	numeric = false,
	className,
	children,
}: Readonly<{
	sort?: SortDirection;
	onSort: () => void;
	numeric?: boolean;
	className?: string;
	children: ReactNode;
}>) {
	const Icon =
		sort === "ascending"
			? ChevronUp
			: sort === "descending"
				? ChevronDown
				: ChevronsUpDown;
	return (
		<th
			scope="col"
			aria-sort={sort}
			className={cx(HEAD_CELL, numeric && "text-right", className)}
		>
			<button
				type="button"
				onClick={onSort}
				className="inline-flex items-center gap-0.5 uppercase tracking-[inherit] hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
			>
				{children}
				<Icon
					aria-hidden
					className={cx("size-3", sort === "none" && "opacity-50")}
				/>
			</button>
		</th>
	);
}

export function Tr({
	selected = false,
	dim = false,
	className,
	...props
}: ComponentProps<"tr"> & { selected?: boolean; dim?: boolean }) {
	return (
		<tr
			aria-selected={selected || undefined}
			data-dim={dim || undefined}
			className={cx(
				"hover:bg-row-hover [&:first-child>td]:border-t-0",
				selected && "bg-row-selected hover:bg-row-selected",
				dim && "text-muted-foreground",
				className,
			)}
			{...props}
		/>
	);
}

export type CellKind = "text" | "name" | "mono" | "num" | "act" | "more";

const CELL_KIND: Record<CellKind, string> = {
	text: "wrap-anywhere",
	name: "wrap-normal",
	mono: "truncate font-mono wrap-normal",
	num: "text-right font-mono tabular-nums wrap-normal",
	act: "min-w-[170px]",
	more: "pr-2 pl-1 text-right",
};

export interface TdProps extends Omit<ComponentProps<"td">, "children"> {
	/** Column name shown above the value when the table stacks into cards (SPEC §4.18). */
	label: string;
	kind?: CellKind;
	children?: ReactNode;
}

export function Td({ label, kind = "text", className, ...props }: TdProps) {
	return (
		<td
			data-label={label}
			data-cell={kind === "act" || kind === "more" ? kind : undefined}
			className={cx(
				"min-w-0 border-t border-hairline px-3 py-2 align-top first:pl-4 last:pr-4",
				CELL_KIND[kind],
				className,
			)}
			{...props}
		/>
	);
}

/** A 12 px muted second line inside a cell (`.sub`). */
export function CellSub({ className, ...props }: ComponentProps<"span">) {
	return (
		<span
			className={cx("mt-0.5 block text-xs text-muted-foreground", className)}
			{...props}
		/>
	);
}

/** Sunken full-width group header (device name + presence + stamp on N4). */
export function GroupRow({
	colSpan,
	className,
	children,
}: Readonly<{ colSpan: number; className?: string; children: ReactNode }>) {
	return (
		<tr data-group="" className="hover:bg-transparent">
			<td
				colSpan={colSpan}
				className={cx(
					"border-t border-hairline bg-surface-sunken px-4 py-1.5 text-xs text-ink-2",
					className,
				)}
			>
				<div className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
					{children}
				</div>
			</td>
		</tr>
	);
}
