"use client";

import { ChevronRight, Flame, List } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../../ui";
import { EmptyState } from "./telemetry-shared";
import { formatDurationMs, isErrorStatus } from "./traces-shared";
import type { ITelemetryTraceSpan } from "./types";

const ROW_HEIGHT = 24;
const MIN_LABEL_PX = 44;
const MIN_BAR_PERCENT = 0.15;
const TRACK_MIN_WIDTH = 720;
const TOOLTIP_WIDTH = 288;
const TOOLTIP_HEIGHT = 160;

interface FlameNode {
	key: string;
	span: ITelemetryTraceSpan;
	start: number;
	end: number;
	depth: number;
	parent: FlameNode | null;
	children: FlameNode[];
}

interface FlameTree {
	nodes: FlameNode[];
	byKey: Map<string, FlameNode>;
	totalMs: number;
}

function spanStartMs(span: ITelemetryTraceSpan): number {
	const parsed = new Date(span.startedAt).getTime();
	return Number.isNaN(parsed) ? 0 : parsed;
}

function buildFlameTree(spans: ITelemetryTraceSpan[]): FlameTree {
	if (spans.length === 0) {
		return { nodes: [], byKey: new Map(), totalMs: 0 };
	}

	const sorted = [...spans].sort((a, b) => spanStartMs(a) - spanStartMs(b));
	const origin = spanStartMs(sorted[0]);

	const bySpanId = new Map<string, FlameNode>();
	const byKey = new Map<string, FlameNode>();
	for (const span of sorted) {
		const duration = Math.max(0, span.durationMs ?? 0);
		const start = spanStartMs(span) - origin;
		const node: FlameNode = {
			key: span.id || span.spanId,
			span,
			start,
			end: start + duration,
			depth: 0,
			parent: null,
			children: [],
		};
		byKey.set(node.key, node);
		if (!bySpanId.has(span.spanId)) bySpanId.set(span.spanId, node);
	}

	const roots: FlameNode[] = [];
	for (const node of byKey.values()) {
		const parentId = node.span.parentSpanId;
		const parent = parentId ? bySpanId.get(parentId) : undefined;
		if (parent && parent !== node) {
			node.parent = parent;
			parent.children.push(node);
		} else {
			roots.push(node);
		}
	}

	const flattened: FlameNode[] = [];
	const visited = new Set<string>();
	const walk = (node: FlameNode, depth: number) => {
		if (visited.has(node.key)) return;
		visited.add(node.key);
		node.depth = depth;
		flattened.push(node);
		for (const child of [...node.children].sort((a, b) => a.start - b.start)) {
			walk(child, depth + 1);
		}
	};
	for (const root of roots.sort((a, b) => a.start - b.start)) walk(root, 0);
	for (const node of byKey.values()) {
		if (!visited.has(node.key)) {
			node.parent = null;
			walk(node, 0);
		}
	}

	const totalMs = flattened.reduce((max, node) => Math.max(max, node.end), 0);
	return { nodes: flattened, byKey, totalMs };
}

function subtreeOf(root: FlameNode): FlameNode[] {
	const out: FlameNode[] = [];
	const seen = new Set<string>();
	const stack = [root];
	while (stack.length > 0) {
		const node = stack.pop();
		if (!node || seen.has(node.key)) continue;
		seen.add(node.key);
		out.push(node);
		for (const child of node.children) stack.push(child);
	}
	return out.sort((a, b) =>
		a.depth === b.depth ? a.start - b.start : a.depth - b.depth,
	);
}

function ancestorsOf(node: FlameNode): FlameNode[] {
	const chain: FlameNode[] = [];
	let current: FlameNode | null = node;
	const guard = new Set<string>();
	while (current && !guard.has(current.key)) {
		guard.add(current.key);
		chain.unshift(current);
		current = current.parent;
	}
	return chain;
}

function barBackground(intensity: number, error: boolean): string {
	const hue = error ? "var(--destructive)" : "var(--chart-1)";
	const mix = error ? 48 : 12 + intensity * 33;
	return `color-mix(in oklab, ${hue} ${mix.toFixed(1)}%, var(--card))`;
}

function attributeEntries(
	attributes: Record<string, unknown> | null | undefined,
): { key: string; value: string }[] {
	if (!attributes) return [];
	return Object.entries(attributes)
		.slice(0, 8)
		.map(([key, value]) => {
			const raw = typeof value === "string" ? value : JSON.stringify(value);
			const text = raw ?? "null";
			return {
				key,
				value: text.length > 120 ? `${text.slice(0, 117)}…` : text,
			};
		});
}

function SpanTooltip({
	node,
	x,
	y,
}: {
	readonly node: FlameNode;
	readonly x: number;
	readonly y: number;
}) {
	const attributes = attributeEntries(node.span.attributes);
	return (
		<div
			className="pointer-events-none absolute z-50 w-72 rounded-lg border bg-popover p-3 shadow-lg"
			style={{ left: x, top: y }}
		>
			<div className="truncate font-mono text-xs font-semibold text-foreground">
				{node.span.name}
			</div>
			<div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
				<span className="tabular-nums text-foreground">
					{formatDurationMs(node.span.durationMs)}
				</span>
				<span>{node.span.kind}</span>
				<span>{node.span.source}</span>
				<span
					className={
						isErrorStatus(node.span.status)
							? "font-medium text-destructive"
							: "font-medium text-foreground"
					}
				>
					{node.span.status}
				</span>
				<span className="tabular-nums">
					+{formatDurationMs(node.start)} in trace
				</span>
			</div>
			{attributes.length > 0 ? (
				<dl className="mt-2 space-y-0.5 border-t pt-2 text-[11px]">
					{attributes.map((attribute) => (
						<div key={attribute.key} className="flex gap-2">
							<dt className="shrink-0 font-mono text-muted-foreground">
								{attribute.key}
							</dt>
							<dd className="min-w-0 flex-1 truncate font-mono text-foreground">
								{attribute.value}
							</dd>
						</div>
					))}
				</dl>
			) : null}
		</div>
	);
}

function FlameRuler({
	viewStart,
	viewSpan,
}: {
	readonly viewStart: number;
	readonly viewSpan: number;
}) {
	const ticks = [0, 0.25, 0.5, 0.75, 1];
	return (
		<div className="relative mb-1 h-4 border-b border-dashed">
			{ticks.map((tick) => (
				<span
					key={tick}
					className="absolute top-0 text-[10px] tabular-nums text-muted-foreground"
					style={{
						left: `${tick * 100}%`,
						transform:
							tick === 0
								? "none"
								: tick === 1
									? "translateX(-100%)"
									: "translateX(-50%)",
					}}
				>
					{formatDurationMs(viewStart + viewSpan * tick)}
				</span>
			))}
		</div>
	);
}

interface TraceFlamegraphProps {
	spans: ITelemetryTraceSpan[];
	className?: string;
}

export function TraceFlamegraph({
	spans,
	className,
}: Readonly<TraceFlamegraphProps>) {
	const tree = useMemo(() => buildFlameTree(spans), [spans]);
	const [focusKey, setFocusKey] = useState<string | null>(null);
	const [view, setView] = useState<"flame" | "list">("flame");
	const [hovered, setHovered] = useState<{
		node: FlameNode;
		x: number;
		y: number;
	} | null>(null);
	const rootRef = useRef<HTMLDivElement | null>(null);
	const trackRef = useRef<HTMLDivElement | null>(null);
	const [trackWidth, setTrackWidth] = useState(TRACK_MIN_WIDTH);
	const [renderedTree, setRenderedTree] = useState(tree);

	if (renderedTree !== tree) {
		setRenderedTree(tree);
		setFocusKey(null);
		setHovered(null);
	}

	useEffect(() => {
		const element = trackRef.current;
		if (!element || typeof ResizeObserver === "undefined") return;
		const observer = new ResizeObserver((entries) => {
			for (const entry of entries) {
				setTrackWidth(Math.max(TRACK_MIN_WIDTH, entry.contentRect.width));
			}
		});
		observer.observe(element);
		return () => observer.disconnect();
	}, []);

	const focus = focusKey ? (tree.byKey.get(focusKey) ?? null) : null;
	const visible = useMemo(
		() => (focus ? subtreeOf(focus) : tree.nodes),
		[focus, tree.nodes],
	);

	const viewStart = focus ? focus.start : 0;
	const viewSpan = Math.max(
		focus ? focus.end - focus.start : tree.totalMs,
		0.001,
	);
	const baseDepth = focus ? focus.depth : 0;
	const maxRelativeDepth = visible.reduce(
		(max, node) => Math.max(max, node.depth - baseDepth),
		0,
	);
	const maxDuration = visible.reduce(
		(max, node) => Math.max(max, node.end - node.start),
		0.001,
	);
	const breadcrumb = focus ? ancestorsOf(focus) : [];

	const showTooltipAt = useCallback(
		(node: FlameNode, clientX: number, clientY: number) => {
			const rect = rootRef.current?.getBoundingClientRect();
			if (!rect) return;
			const x = clientX - rect.left + 12;
			const y = clientY - rect.top + 16;
			setHovered({
				node,
				x: Math.max(4, Math.min(x, Math.max(4, rect.width - TOOLTIP_WIDTH))),
				y:
					y + TOOLTIP_HEIGHT > rect.height
						? Math.max(4, y - TOOLTIP_HEIGHT)
						: y,
			});
		},
		[],
	);

	const hideTooltip = useCallback(() => setHovered(null), []);

	if (tree.nodes.length === 0) {
		return (
			<EmptyState
				message="This trace has no spans to visualise."
				className="py-10 text-sm"
			/>
		);
	}

	return (
		<div ref={rootRef} className={`relative ${className ?? ""}`}>
			<div className="mb-2 flex flex-wrap items-center justify-between gap-2">
				<nav
					aria-label="Flamegraph zoom path"
					className="flex min-w-0 flex-wrap items-center gap-1 text-xs"
				>
					<Button
						variant={focus ? "ghost" : "secondary"}
						size="sm"
						className="h-6 px-2 text-xs"
						onClick={() => setFocusKey(null)}
					>
						Full trace
					</Button>
					{breadcrumb.map((node, index) => (
						<span key={node.key} className="flex min-w-0 items-center gap-1">
							<ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
							<Button
								variant={
									index === breadcrumb.length - 1 ? "secondary" : "ghost"
								}
								size="sm"
								className="h-6 max-w-[12rem] justify-start truncate px-2 font-mono text-xs"
								onClick={() => setFocusKey(node.key)}
							>
								{node.span.name}
							</Button>
						</span>
					))}
				</nav>
				<div className="flex items-center gap-1">
					<Button
						variant={view === "flame" ? "secondary" : "ghost"}
						size="sm"
						className="h-7 px-2 text-xs"
						onClick={() => setView("flame")}
					>
						<Flame className="mr-1 h-3 w-3" />
						Flame
					</Button>
					<Button
						variant={view === "list" ? "secondary" : "ghost"}
						size="sm"
						className="h-7 px-2 text-xs"
						onClick={() => setView("list")}
					>
						<List className="mr-1 h-3 w-3" />
						List
					</Button>
				</div>
			</div>

			{view === "flame" ? (
				<div className="w-full overflow-x-auto rounded-lg border bg-card/40 p-3">
					<div ref={trackRef} style={{ minWidth: TRACK_MIN_WIDTH }}>
						<FlameRuler viewStart={viewStart} viewSpan={viewSpan} />
						<div
							className="relative"
							style={{ height: (maxRelativeDepth + 1) * ROW_HEIGHT }}
						>
							{visible.map((node) => {
								const duration = node.end - node.start;
								const rawLeft = ((node.start - viewStart) / viewSpan) * 100;
								const left = Math.max(0, Math.min(100, rawLeft));
								const width = Math.max(
									MIN_BAR_PERCENT,
									Math.min(100 - left, (duration / viewSpan) * 100),
								);
								const error = isErrorStatus(node.span.status);
								const intensity = Math.sqrt(
									Math.min(1, duration / maxDuration),
								);
								const showLabel = (width / 100) * trackWidth >= MIN_LABEL_PX;
								return (
									<button
										type="button"
										key={node.key}
										aria-label={`${node.span.name}, ${formatDurationMs(duration)}, ${node.span.status}`}
										className="absolute overflow-hidden rounded-sm border border-border/60 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
										style={{
											left: `${left}%`,
											width: `${width}%`,
											top: (node.depth - baseDepth) * ROW_HEIGHT,
											height: ROW_HEIGHT - 3,
											minWidth: 2,
											background: barBackground(intensity, error),
											borderColor: error
												? "color-mix(in oklab, var(--destructive) 45%, transparent)"
												: undefined,
										}}
										onClick={() => setFocusKey(node.key)}
										onMouseEnter={(event) =>
											showTooltipAt(node, event.clientX, event.clientY)
										}
										onMouseMove={(event) =>
											showTooltipAt(node, event.clientX, event.clientY)
										}
										onMouseLeave={hideTooltip}
										onFocus={(event) => {
											const rect = event.currentTarget.getBoundingClientRect();
											showTooltipAt(node, rect.left, rect.bottom - 8);
										}}
										onBlur={hideTooltip}
									>
										{showLabel ? (
											<span className="block truncate px-1.5 text-[11px] leading-[21px] text-foreground">
												{node.span.name}
											</span>
										) : null}
									</button>
								);
							})}
						</div>
					</div>
				</div>
			) : (
				<ul className="divide-y rounded-lg border">
					{visible.map((node) => {
						const duration = node.end - node.start;
						const error = isErrorStatus(node.span.status);
						return (
							<li key={node.key}>
								<button
									type="button"
									onClick={() => setFocusKey(node.key)}
									className="flex w-full items-center gap-3 px-3 py-1.5 text-left hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
								>
									<span
										className="min-w-0 flex-1 truncate font-mono text-xs text-foreground"
										style={{
											paddingLeft: (node.depth - baseDepth) * 14,
										}}
									>
										{node.span.name}
									</span>
									<span className="shrink-0 text-[11px] text-muted-foreground">
										{node.span.kind}
									</span>
									<span
										className={`w-16 shrink-0 text-right text-[11px] font-medium ${error ? "text-destructive" : "text-muted-foreground"}`}
									>
										{node.span.status}
									</span>
									<span className="w-20 shrink-0 text-right text-xs tabular-nums text-foreground">
										{formatDurationMs(duration)}
									</span>
								</button>
							</li>
						);
					})}
				</ul>
			)}

			<p className="mt-2 text-[11px] text-muted-foreground">
				Bar width is the span duration, colour intensity its share of the
				slowest span in view. Select a span to zoom into its subtree.
			</p>

			{hovered ? (
				<SpanTooltip node={hovered.node} x={hovered.x} y={hovered.y} />
			) : null}
		</div>
	);
}
