"use client";

import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { resolveChildSpecs } from "../children";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { GridComponent } from "../types";

export function A2UIGrid({
	component,
	style,
	renderChild,
}: ComponentProps<GridComponent>) {
	const { resolve } = useData();

	const columns = component.columns
		? (resolve(component.columns) as string | number)
		: undefined;
	const rows = component.rows
		? (resolve(component.rows) as string | number)
		: undefined;
	const gap = component.gap ? (resolve(component.gap) as string) : undefined;
	const columnGap = component.columnGap
		? (resolve(component.columnGap) as string)
		: undefined;
	const rowGap = component.rowGap
		? (resolve(component.rowGap) as string)
		: undefined;
	const autoFlow = component.autoFlow
		? (resolve(component.autoFlow) as string)
		: undefined;

	const autoFlowMap: Record<string, string> = {
		row: "grid-flow-row",
		column: "grid-flow-col",
		dense: "grid-flow-dense",
		rowDense: "grid-flow-row-dense",
		columnDense: "grid-flow-col-dense",
	};

	const children = resolveChildSpecs(component.children, resolve);

	const gridStyle: React.CSSProperties = {
		...resolveInlineStyle(style),
	};

	if (typeof columns === "number") {
		gridStyle.gridTemplateColumns = `repeat(${columns}, minmax(0, 1fr))`;
	} else if (columns) {
		gridStyle.gridTemplateColumns = columns;
	}

	if (typeof rows === "number") {
		gridStyle.gridTemplateRows = `repeat(${rows}, minmax(0, 1fr))`;
	} else if (rows) {
		gridStyle.gridTemplateRows = rows;
	}

	if (gap) gridStyle.gap = gap;
	if (columnGap) gridStyle.columnGap = columnGap;
	if (rowGap) gridStyle.rowGap = rowGap;

	return (
		<div
			className={cn(
				"grid",
				autoFlow && autoFlowMap[autoFlow],
				resolveStyle(style),
			)}
			style={gridStyle}
		>
			{children.map((child) => (
				<Fragment key={child.key}>
					{renderChild(child.id, child.scope)}
				</Fragment>
			))}
		</div>
	);
}
