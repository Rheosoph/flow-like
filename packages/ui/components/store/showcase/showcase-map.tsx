"use client";

import { useTranslation } from "@flow-like/locales";
import { Info, Network } from "lucide-react";
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import {
	type ShowcaseGraph,
	type ShowcaseLaneKey,
	type ShowcaseMapNode,
	compact,
	eventKind,
	nodeIcon,
	traceSet,
} from "./model";
import { NODE_TONE, ShowcaseSection } from "./showcase-ui";

interface EdgePath {
	from: string;
	to: string;
	d: string;
}

const LANE_TONE: Record<ShowcaseLaneKey, string> = {
	trig: "text-primary",
	flow: "text-muted-foreground",
	data: "text-[var(--showcase-text)]",
	surf: "text-tertiary",
};

export function useNodeSub() {
	const { t } = useTranslation("store");
	return useCallback(
		(n: ShowcaseMapNode): string => {
			switch (n.kind) {
				case "event": {
					const [key, fallback] = eventKind(n.eventType ?? "").label;
					return t(key as "showcaseEventOther", fallback);
				}
				case "flow":
					return t("showcaseNodeCount", {
						defaultValue_one: "{{count}} node",
						defaultValue_other: "{{count}} nodes",
						count: n.nodeCount ?? 0,
					});
				case "table":
					return typeof n.rows === "number"
						? t("showcaseRowCount", {
								defaultValue_one: "{{formattedCount}} row",
								defaultValue_other: "{{formattedCount}} rows",
								count: n.rows,
								formattedCount: compact(n.rows),
							})
						: t("showcaseTable", "Table");
				case "source":
					return n.driver ?? t("showcaseConnection", "Connection");
				case "page":
					return n.route || t("showcasePage", "Page");
				default:
					return t("showcaseHomeWidget", "Home widget");
			}
		},
		[t],
	);
}

function MapNodeButton({
	node,
	related,
	dimmed,
	nodeRef,
	onTrace,
	onOpen,
}: Readonly<{
	node: ShowcaseMapNode;
	related: boolean;
	dimmed: boolean;
	nodeRef: (el: HTMLButtonElement | null) => void;
	onTrace: (key: string | null) => void;
	onOpen: (node: ShowcaseMapNode) => void;
}>) {
	const sub = useNodeSub()(node);
	const Icon = nodeIcon(node.kind, node.eventType);
	return (
		<button
			type="button"
			ref={nodeRef}
			data-node={node.key}
			data-related={related || undefined}
			onClick={() => onOpen(node)}
			onPointerEnter={() => onTrace(node.key)}
			onPointerLeave={() => onTrace(null)}
			onFocus={() => onTrace(node.key)}
			onBlur={() => onTrace(null)}
			className={cn(
				"flex w-full min-w-0 items-center gap-2.5 rounded-[10px] border border-border bg-card px-2.5 py-2 text-left transition-[opacity,border-color] hover:border-foreground/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
				dimmed && "opacity-30",
				related && "border-foreground/30",
			)}
		>
			<span
				className={cn(
					"grid h-7 w-7 shrink-0 place-items-center rounded-md bg-muted",
					NODE_TONE[node.kind],
				)}
			>
				<Icon className="h-4 w-4" aria-hidden="true" />
			</span>
			<span className="flex min-w-0 flex-col">
				<b
					className={cn(
						"wrap-anywhere text-[13px] leading-tight font-medium",
						node.mono && "font-mono text-[12.5px]",
					)}
				>
					{node.label}
				</b>
				<small className="wrap-anywhere text-[11.5px] text-muted-foreground">
					{sub}
				</small>
			</span>
		</button>
	);
}

export function ShowcaseMap({
	graph,
	onOpen,
}: Readonly<{
	graph: ShowcaseGraph;
	onOpen: (node: ShowcaseMapNode) => void;
}>) {
	const { t } = useTranslation("store");
	const containerRef = useRef<HTMLDivElement>(null);
	const nodeEls = useRef(new Map<string, HTMLButtonElement>());
	const [paths, setPaths] = useState<EdgePath[]>([]);
	const [size, setSize] = useState({ w: 0, h: 0 });
	const [active, setActive] = useState<string | null>(null);

	const laneTitle: Record<ShowcaseLaneKey, string> = {
		trig: t("showcaseLaneStartsFrom", "Starts from"),
		flow: t("showcaseLaneRuns", "Runs"),
		data: t("showcaseLaneWorksWith", "Works with"),
		surf: t("showcaseLaneShowsUpAs", "Shows up as"),
	};

	const related = useMemo(
		() => (active ? traceSet(graph.edges, active) : null),
		[active, graph.edges],
	);

	const measure = useCallback(() => {
		const box = containerRef.current;
		if (!box) return;
		const mr = box.getBoundingClientRect();
		const next: EdgePath[] = [];
		for (const e of graph.edges) {
			const a = nodeEls.current.get(e.from)?.getBoundingClientRect();
			const b = nodeEls.current.get(e.to)?.getBoundingClientRect();
			if (!a || !b) continue;
			const x1 = a.right - mr.left;
			const x2 = b.left - mr.left;
			const y1 = a.top - mr.top + a.height / 2;
			const y2 = b.top - mr.top + b.height / 2;
			const dx = Math.max(24, (x2 - x1) * 0.5);
			next.push({
				from: e.from,
				to: e.to,
				d: `M${x1},${y1} C${x1 + dx},${y1} ${x2 - dx},${y2} ${x2},${y2}`,
			});
		}
		setSize({ w: mr.width, h: mr.height });
		setPaths(next);
	}, [graph.edges]);

	useLayoutEffect(() => {
		measure();
		const box = containerRef.current;
		if (!box || typeof ResizeObserver === "undefined") return;
		const ro = new ResizeObserver(() => measure());
		ro.observe(box);
		return () => ro.disconnect();
	}, [measure]);

	return (
		<ShowcaseSection
			id="map"
			icon={Network}
			eyebrow={t("showcaseMapEyebrow", "How it works")}
			title={t("showcaseMapTitle", "From trigger to result")}
		>
			<div className="-m-1 overflow-x-auto p-1">
				<div
					ref={containerRef}
					data-showcase-map
					className="relative grid gap-x-14 rounded-xl border border-border bg-card p-4.5 bg-[radial-gradient(circle_at_1px_1px,color-mix(in_oklch,var(--muted-foreground)_22%,transparent)_1px,transparent_0)] bg-size-[18px_18px]"
					style={{
						gridTemplateColumns: `repeat(${graph.lanes.length}, minmax(180px, 1fr))`,
						minWidth: `${graph.lanes.length * 220}px`,
					}}
				>
					<svg
						className="pointer-events-none absolute inset-0 z-0 overflow-visible"
						width={size.w}
						height={size.h}
						viewBox={`0 0 ${size.w || 1} ${size.h || 1}`}
						aria-hidden="true"
					>
						{paths.map((p) => {
							const hot = !!related && related.has(p.from) && related.has(p.to);
							return (
								<path
									key={`${p.from}>${p.to}`}
									data-edge={`${p.from}>${p.to}`}
									data-hot={hot || undefined}
									d={p.d}
									fill="none"
									className={cn(
										"transition-opacity",
										hot
											? "stroke-primary opacity-100 motion-safe:animate-[showcase-flow_0.9s_linear_infinite]"
											: "stroke-muted-foreground/40",
										related && !hot && "opacity-15",
									)}
									strokeWidth={hot ? 2 : 1.5}
									strokeDasharray={hot ? "6 6" : undefined}
								/>
							);
						})}
					</svg>
					{graph.lanes.map((lane) => (
						<div
							key={lane.key}
							data-lane={lane.key}
							className="relative z-10 flex min-w-0 flex-col gap-2"
						>
							<div
								className={cn(
									"mb-0.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-widest text-muted-foreground",
								)}
							>
								<span
									className={cn(
										"h-2 w-2 rounded-xs bg-current",
										LANE_TONE[lane.key],
									)}
									aria-hidden="true"
								/>
								{laneTitle[lane.key]}
							</div>
							<div className="flex flex-1 flex-col justify-center gap-2">
								{lane.nodes.map((n) => (
									<MapNodeButton
										key={n.key}
										node={n}
										related={!!related?.has(n.key)}
										dimmed={!!related && !related.has(n.key)}
										nodeRef={(el) => {
											if (el) nodeEls.current.set(n.key, el);
											else nodeEls.current.delete(n.key);
										}}
										onTrace={setActive}
										onOpen={onOpen}
									/>
								))}
							</div>
						</div>
					))}
				</div>
			</div>
			<p className="flex items-start gap-1.5 text-xs text-muted-foreground">
				<Info className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
				{t(
					"showcaseMapHint",
					"Hover or focus any part to trace what it touches. Click to open it.",
				)}
			</p>
		</ShowcaseSection>
	);
}
