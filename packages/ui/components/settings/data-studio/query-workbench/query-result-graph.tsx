"use client";

import { useTranslation } from "@flow-like/locales";
import { ResponsiveNetwork } from "@nivo/network";
import { Settings2 } from "lucide-react";
import { useMemo } from "react";
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

const MAX_NODES = 500;
const MAX_LINKS = 1000;

interface NetworkNode {
	id: string;
	color: string;
	size: number;
}

interface NetworkLink {
	source: string;
	target: string;
}

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
	const { data, truncated } = useMemo(() => {
		const source = config.source;
		const target = config.target;
		if (!source || !target) {
			return { data: { nodes: [], links: [] }, truncated: false };
		}
		const degree = new Map<string, number>();
		const links: NetworkLink[] = [];
		let cut = false;
		for (const row of rows) {
			const from = row[source];
			const to = row[target];
			if (
				from === null ||
				from === undefined ||
				to === null ||
				to === undefined
			)
				continue;
			const fromId = String(from);
			const toId = String(to);
			if (degree.size >= MAX_NODES || links.length >= MAX_LINKS) {
				cut = true;
				break;
			}
			degree.set(fromId, (degree.get(fromId) ?? 0) + 1);
			degree.set(toId, (degree.get(toId) ?? 0) + 1);
			links.push({ source: fromId, target: toId });
		}
		let maxDegree = 1;
		for (const value of degree.values()) maxDegree = Math.max(maxDegree, value);
		const nodes: NetworkNode[] = [...degree.entries()].map(([id, value]) => ({
			id,
			color: "var(--chart-1)",
			size: 6 + (value / maxDegree) * 12,
		}));
		return { data: { nodes, links }, truncated: cut };
	}, [rows, config.source, config.target]);

	const update = (partial: Partial<GraphVizConfig>) =>
		onConfigChange({ ...config, ...partial });

	const ready = Boolean(
		config.source && config.target && data.nodes.length > 0,
	);

	return (
		<div className="relative flex h-full min-h-0 flex-col">
			<div className="absolute right-0 top-0 z-10 flex items-center gap-2">
				{truncated && (
					<span className="text-xs text-amber-600 dark:text-amber-400">
						{t("cappedAtMax_nodesNodes", "Capped at {{MAX_NODES}} nodes", {
							MAX_NODES,
						})}
					</span>
				)}
				<Popover>
					<PopoverTrigger asChild>
						<Button variant="outline" size="sm" className="h-8 gap-1.5">
							<Settings2 className="h-3.5 w-3.5" />{" "}
							{t("configure", "Configure")}
						</Button>
					</PopoverTrigger>
					<PopoverContent align="end" className="w-64 space-y-3">
						<div className="grid gap-1.5">
							<Label className="text-xs text-muted-foreground">
								{t("sourceColumn", "Source column")}
							</Label>
							<Select
								value={config.source}
								onValueChange={(value) => update({ source: value })}
							>
								<SelectTrigger
									className="h-8"
									aria-label={t("sourceColumn", "Source column")}
								>
									<SelectValue placeholder="Select" />
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
						<div className="grid gap-1.5">
							<Label className="text-xs text-muted-foreground">
								{t("targetColumn", "Target column")}
							</Label>
							<Select
								value={config.target}
								onValueChange={(value) => update({ target: value })}
							>
								<SelectTrigger
									className="h-8"
									aria-label={t("targetColumn", "Target column")}
								>
									<SelectValue placeholder="Select" />
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
					</PopoverContent>
				</Popover>
			</div>

			{!ready ? (
				<div className="flex h-full items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground">
					{t(
						"mapASourceAndTargetColumnToDrawARelationshipGraph",
						"Map a source and target column to draw a relationship graph.",
					)}
				</div>
			) : (
				<div
					role="img"
					aria-label={t(
						"relationshipGraphWithNodesAndLinks",
						"Relationship graph with {{nodes}} and {{links}}",
						{
							nodes: t("countNodes", {
								defaultValue_one: "{{count}} Node",
								defaultValue_other: "{{count}} Nodes",
								count: data.nodes.length,
							}),
							links: t("countLinks", {
								defaultValue_one: "{{count}} link",
								defaultValue_other: "{{count}} links",
								count: data.links.length,
							}),
						},
					)}
					className="h-full w-full"
				>
					<ResponsiveNetwork
						data={data}
						linkDistance={60}
						centeringStrength={0.3}
						repulsivity={12}
						nodeSize={(node: NetworkNode) => node.size}
						activeNodeSize={(node: NetworkNode) => node.size * 1.5}
						nodeColor={(node: NetworkNode) => node.color}
						nodeBorderWidth={1.5}
						nodeBorderColor="var(--background)"
						linkThickness={1}
						linkColor="var(--border)"
						nodeTooltip={({ node }: { node: { id: string } }) => (
							<div className="rounded-lg border bg-background px-2.5 py-1.5 text-xs shadow-floating">
								<span className="font-mono">{node.id}</span>
							</div>
						)}
					/>
				</div>
			)}
		</div>
	);
}
