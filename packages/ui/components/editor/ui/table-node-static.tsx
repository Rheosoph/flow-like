"use client";
import type * as React from "react";

import type { TTableCellElement, TTableElement } from "platejs";
import type { SlateElementProps } from "platejs/static";

import { BaseTablePlugin } from "@platejs/table";
import { SlateElement } from "platejs/static";

import { cn } from "../../../lib/utils";
import { TableViewer } from "./table-viewer";

const childrenOf = (node: unknown): readonly unknown[] => {
	const children = (node as { children?: unknown } | null)?.children;
	return Array.isArray(children) ? children : [];
};

/** Stored documents come from models too, so malformed nodes must not throw. */
function textOf(node: unknown): string {
	const text = (node as { text?: unknown } | null)?.text;
	if (typeof text === "string") return text;
	return childrenOf(node).map(textOf).join("");
}

function extractTableDataStatic(element: TTableElement): string[][] {
	return childrenOf(element).map((row) => childrenOf(row).map(textOf));
}

export function TableElementStatic({
	children,
	...props
}: SlateElementProps<TTableElement>) {
	const { disableMarginLeft } = props.editor.getOptions(BaseTablePlugin);
	const marginLeft = disableMarginLeft ? 0 : props.element.marginLeft;
	const tableData = extractTableDataStatic(props.element);

	return (
		<SlateElement
			{...props}
			className="py-3"
			style={{ paddingLeft: marginLeft }}
		>
			<TableViewer data={tableData}>{children}</TableViewer>
		</SlateElement>
	);
}

export function TableRowElementStatic(props: SlateElementProps) {
	return (
		<SlateElement {...props} as="tr" className="h-full group/row">
			{props.children}
		</SlateElement>
	);
}

export function TableCellElementStatic({
	isHeader,
	...props
}: SlateElementProps<TTableCellElement> & {
	isHeader?: boolean;
}) {
	const { editor, element } = props;
	const { api } = editor.getPlugin(BaseTablePlugin);

	const { minHeight, width } = api.table.getCellSize({ element });
	const borders = api.table.getCellBorders({ element });

	return (
		<SlateElement
			{...props}
			as={isHeader ? "th" : "td"}
			className={cn(
				"h-full overflow-visible border-none bg-background p-0",
				element.background ? "bg-(--cellBackground)" : "bg-background",
				isHeader && "text-left font-semibold bg-muted/50 *:m-0",
				"before:size-full",
				"before:absolute before:box-border before:content-[''] before:select-none",
				// Always show borders for cleaner look in static mode
				"before:border-b before:border-b-border before:border-r before:border-r-border",
				borders &&
					cn(
						borders.left?.size && `before:border-l before:border-l-border`,
						borders.top?.size && `before:border-t before:border-t-border`,
					),
			)}
			style={
				{
					"--cellBackground": element.background,
					maxWidth: 300,
					minWidth: 60,
				} as React.CSSProperties
			}
			attributes={{
				...props.attributes,
				colSpan: api.table.getColSpan(element),
				rowSpan: api.table.getRowSpan(element),
			}}
		>
			<div className="relative z-20 box-border h-full px-2 py-1.5 text-sm">
				{props.children}
			</div>
		</SlateElement>
	);
}

export function TableCellHeaderElementStatic(
	props: SlateElementProps<TTableCellElement>,
) {
	return <TableCellElementStatic {...props} isHeader />;
}
