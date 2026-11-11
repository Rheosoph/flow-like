"use client";

import { useTranslation } from "@flow-like/locales";
import { Settings2 } from "lucide-react";
import { Suspense, lazy, useMemo } from "react";
import type {
	GraphVizConfig,
	QueryColumn,
} from "../../../../state/backend-state/query-state";
import { Button } from "../../../ui/button";
import { Label } from "../../../ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "../../../ui/popover";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { humanizeIdentifier } from "../data-studio-panels";
import { buildQueryResultGraph } from "./query-result-graph-model";

// Sigma brings a WebGL renderer and graphology; load them only once the Graph
// tab opens.
const GraphViewer = lazy(() =>
	import("../../../ui/graph/graph-viewer").then((module) => ({
		default: module.GraphViewer,
	})),
);

export function inferGraphConfig(columns: QueryColumn[]): GraphVizConfig {
	return {
		source: columns[0]?.name,
		target: columns[1]?.name ?? columns[0]?.name,
	};
}

export function QueryResultGraph({
	columns,
	rows,
	config,
	onConfigChange,
}: Readonly<{
	columns: QueryColumn[];
	rows: Record<string, unknown>[];
	config: GraphVizConfig;
	onConfigChange: (config: GraphVizConfig) => void;
}>) {
	const { t } = useTranslation("settings");
	const { source, target } = config;
	const graph = useMemo(
		() =>
			source && target
				? buildQueryResultGraph(
						rows,
						{ column: source, label: humanizeIdentifier(source) },
						{ column: target, label: humanizeIdentifier(target) },
					)
				: null,
		[rows, source, target],
	);

	const configure = (
		<GraphColumnsPopover
			columns={columns}
			config={config}
			onConfigChange={onConfigChange}
		/>
	);

	if (!graph || graph.data.nodes.length === 0) {
		return (
			<div className="flex h-full min-h-0 flex-col gap-2">
				<div className="flex justify-end">{configure}</div>
				<div className="flex flex-1 items-center justify-center rounded-lg border border-dashed p-4 text-center text-sm text-muted-foreground">
					{t(
						"mapASourceAndTargetColumnToDrawARelationshipGraph",
						"Map a source and target column to draw a relationship graph.",
					)}
				</div>
			</div>
		);
	}

	return (
		<div className="relative h-full min-h-96 overflow-hidden rounded-lg border bg-background">
			<Suspense
				fallback={<div className="h-full w-full animate-pulse bg-muted/30" />}
			>
				<GraphViewer
					overlay={graph.overlay}
					data={graph.data}
					truncated={graph.data.truncated}
					toolbarActions={configure}
				/>
			</Suspense>
		</div>
	);
}

function GraphColumnsPopover({
	columns,
	config,
	onConfigChange,
}: Readonly<{
	columns: QueryColumn[];
	config: GraphVizConfig;
	onConfigChange: (config: GraphVizConfig) => void;
}>) {
	const { t } = useTranslation("settings");
	const update = (partial: Partial<GraphVizConfig>) =>
		onConfigChange({ ...config, ...partial });

	return (
		<Popover>
			<PopoverTrigger asChild>
				<Button variant="outline" size="sm" className="h-8 shrink-0 gap-1.5">
					<Settings2 className="h-3.5 w-3.5" />
					{t("configure", "Configure")}
				</Button>
			</PopoverTrigger>
			<PopoverContent align="end" className="w-64 space-y-3">
				<ColumnSelect
					label={t("sourceColumn", "Source column")}
					columns={columns}
					value={config.source}
					onChange={(value) => update({ source: value })}
				/>
				<ColumnSelect
					label={t("targetColumn", "Target column")}
					columns={columns}
					value={config.target}
					onChange={(value) => update({ target: value })}
				/>
			</PopoverContent>
		</Popover>
	);
}

function ColumnSelect({
	label,
	columns,
	value,
	onChange,
}: Readonly<{
	label: string;
	columns: QueryColumn[];
	value: string | undefined;
	onChange: (value: string) => void;
}>) {
	return (
		<div className="grid gap-1.5">
			<Label className="text-xs text-muted-foreground">{label}</Label>
			<Select value={value} onValueChange={onChange}>
				<SelectTrigger className="h-8" aria-label={label}>
					<SelectValue />
				</SelectTrigger>
				<SelectContent>
					{columns.map((column) => (
						<SelectItem key={column.name} value={column.name}>
							{humanizeIdentifier(column.name)}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		</div>
	);
}
