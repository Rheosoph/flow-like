"use client";

import { useTranslation } from "@flow-like/locales";
import type { Node } from "@xyflow/react";
import { Maximize, Scan, Search } from "lucide-react";
import { useMemo, useState } from "react";
import { Input } from "../ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { useReactFlow, useStore } from "./board-runtime";

const LARGE_GRAPH_THRESHOLD = 500;
const SEARCH_RESULT_LIMIT = 25;

function nodeLabel(node: Node) {
	const entity = (node.data.node ?? node.data.layer) as
		| { friendly_name?: string; name?: string }
		| undefined;
	return (
		entity?.friendly_name ||
		entity?.name ||
		(typeof node.data.label === "string" ? node.data.label : node.id)
	);
}

/** Keep camera subscriptions here so panning does not rebuild the canvas scene. */
export function LargeGraphNavigation({
	nodes,
	native,
}: { nodes: readonly Node[]; native: boolean }) {
	const { t } = useTranslation("flow");
	const flow = useReactFlow();
	const overview = useStore((state) => state.transform[2] < 0.5);
	const hasSelection = useStore((state) =>
		state.nodes.some((node) => node.selected && !node.hidden),
	);
	const [open, setOpen] = useState(false);
	const [query, setQuery] = useState("");
	const entries = useMemo(
		() =>
			nodes
				.filter((node) => !node.hidden)
				.map((node) => ({ id: node.id, label: nodeLabel(node) }))
				.sort((a, b) => a.label.localeCompare(b.label)),
		[nodes],
	);
	const matches = useMemo(() => {
		const search = query.trim().toLocaleLowerCase();
		return search
			? entries.filter((entry) =>
					`${entry.label} ${entry.id}`.toLocaleLowerCase().includes(search),
				)
			: entries;
	}, [entries, query]);
	if (entries.length < LARGE_GRAPH_THRESHOLD) return null;

	const focus = (id: string) => {
		const node = flow.getInternalNode(id);
		if (!node || node.hidden) return;
		const { x, y } = node.internals.positionAbsolute;
		const width = node.measured.width ?? node.width ?? 0;
		const height = node.measured.height ?? node.height ?? 0;
		setOpen(false);
		void flow.setCenter(x + width / 2, y + height / 2, {
			zoom: 1,
			duration: 250,
		});
	};
	const buttonClass =
		"inline-flex items-center gap-1.5 rounded px-2 py-1 text-xs hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";
	return (
		<div
			className="nodrag nopan nowheel absolute right-3 top-14 z-40 max-w-72 rounded-md border border-border bg-card p-2 text-foreground shadow-sm"
			data-large-graph-navigation
			data-board-dom-control
		>
			<p className="px-1 text-xs font-medium">
				{overview
					? t("largeGraphOverview", "Overview · {{nodeCount}} nodes", {
							nodeCount: entries.length.toLocaleString(),
						})
					: t("largeGraphNodeCount", "{{nodeCount}} nodes", {
							nodeCount: entries.length.toLocaleString(),
						})}
			</p>
			{overview && (
				<p className="mt-1 px-1 text-xs text-muted-foreground">
					{native
						? t(
								"largeGraphOverviewDetail",
								"Node details are reduced at this zoom. Browse or select a node to read it at full size.",
							)
						: t(
								"largeGraphZoomDetail",
								"Browse or select a node to read it at full size.",
							)}
				</p>
			)}
			<div className="mt-1 flex flex-wrap items-center gap-1">
				<Popover open={open} onOpenChange={setOpen}>
					<PopoverTrigger asChild>
						<button type="button" className={buttonClass}>
							<Search className="size-3.5" aria-hidden />
							{t("largeGraphBrowseNodes", "Browse nodes")}
						</button>
					</PopoverTrigger>
					<PopoverContent
						align="end"
						className="w-80 p-2"
						aria-label={t("largeGraphSearchNodes", "Search nodes")}
					>
						<Input
							value={query}
							onChange={(event) => setQuery(event.target.value)}
							placeholder={t(
								"largeGraphSearchPlaceholder",
								"Search by node name or ID",
							)}
							aria-label={t("largeGraphSearchNodes", "Search nodes")}
						/>
						<output className="block px-1 py-2 text-xs text-muted-foreground">
							{t(
								"largeGraphSearchResults",
								"Results: {{shown}} of {{total}}",
								{
									shown: Math.min(matches.length, SEARCH_RESULT_LIMIT),
									total: matches.length.toLocaleString(),
								},
							)}
						</output>
						<ul className="max-h-72 overflow-y-auto">
							{matches.slice(0, SEARCH_RESULT_LIMIT).map((entry) => (
								<li key={entry.id}>
									<button
										type="button"
										className="w-full rounded px-2 py-1.5 text-left hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
										onClick={() => focus(entry.id)}
									>
										<span className="block truncate text-sm">
											{entry.label}
										</span>
										<span className="block truncate font-mono text-[10px] text-muted-foreground">
											{entry.id}
										</span>
									</button>
								</li>
							))}
						</ul>
					</PopoverContent>
				</Popover>
				{hasSelection && (
					<button
						type="button"
						className={buttonClass}
						onClick={() => {
							const selected = flow
								.getNodes()
								.find((node) => node.selected && !node.hidden);
							if (selected) focus(selected.id);
						}}
					>
						<Scan className="size-3.5" aria-hidden />
						{t("largeGraphFocusSelected", "Focus selected node")}
					</button>
				)}
				{!overview && (
					<button
						type="button"
						className={buttonClass}
						onClick={() => void flow.fitView({ duration: 250 })}
					>
						<Maximize className="size-3.5" aria-hidden />
						{t("largeGraphFitGraph", "Fit graph")}
					</button>
				)}
			</div>
		</div>
	);
}
