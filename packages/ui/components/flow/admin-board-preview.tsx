"use client";

import {
	Background,
	BackgroundVariant,
	type NodeProps,
	ReactFlow,
	type ReactFlowInstance,
	type Node as ReactFlowNode,
	ReactFlowProvider,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import {
	ActivityIcon,
	ChevronRight,
	CircleCheckIcon,
	CoinsIcon,
	Crosshair,
	Info,
	Layers,
	LockIcon,
	type LucideIcon,
	ScaleIcon,
	ShieldIcon,
	Variable,
} from "lucide-react";
import { useTheme } from "next-themes";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	type IBoard,
	type INode,
	type IVariable,
	cn,
	parseBoard,
} from "../../lib";
import type { ILayer, ILayerType } from "../../lib/schema/flow/board";
import {
	Badge,
	ScrollArea,
	Separator,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "../ui";
import { CallFunctionNode } from "./call-function-node";
import {
	CommentNode,
	type CommentNode as CommentNodeType,
} from "./comment-node";
import { FlowBreadCrumb } from "./flow-breadcrumb";
import { FlowNode, type FlowNode as FlowNodeType } from "./flow-node";
import { type ILayerInnerNode, LayerInnerNode } from "./layer-inner-node";
import { LayerNode, type LayerNode as LayerNodeType } from "./layer-node";

const SCORE_CATEGORIES = [
	"security",
	"privacy",
	"performance",
	"governance",
	"reliability",
	"cost",
] as const;

type ScoreCategory = (typeof SCORE_CATEGORIES)[number];
type NodeScores = NonNullable<INode["scores"]>;
type ScoreAggregate = {
	scores: Record<ScoreCategory, number>;
	worstScore: number;
	worstCategory: ScoreCategory;
	scoredNodeCount: number;
	nodeCount: number;
};
type NodeScoreSummary = {
	node: INode;
	scores: NodeScores;
	worstScore: number;
	worstCategory: ScoreCategory;
};

const SCORE_LABELS: Record<ScoreCategory, string> = {
	security: "Security",
	privacy: "Privacy",
	performance: "Performance",
	governance: "Governance",
	reliability: "Reliability",
	cost: "Cost",
};

const SCORE_ABBR: Record<ScoreCategory, string> = {
	security: "Sec",
	privacy: "Priv",
	performance: "Perf",
	governance: "Gov",
	reliability: "Rel",
	cost: "Cost",
};

const SCORE_ICONS: Record<ScoreCategory, LucideIcon> = {
	security: ShieldIcon,
	privacy: LockIcon,
	performance: ActivityIcon,
	governance: ScaleIcon,
	reliability: CircleCheckIcon,
	cost: CoinsIcon,
};

function normalizedLayerId(layerId?: string | null) {
	return layerId && layerId !== "" ? layerId : undefined;
}

function scoreTone(value: number) {
	if (value >= 7) {
		return {
			bg: "bg-green-500",
			text: "text-green-600 dark:text-green-400",
			border: "border-green-500/40",
			soft: "bg-green-500/10",
		};
	}
	if (value >= 4) {
		return {
			bg: "bg-yellow-500",
			text: "text-yellow-600 dark:text-yellow-400",
			border: "border-yellow-500/40",
			soft: "bg-yellow-500/10",
		};
	}
	return {
		bg: "bg-red-500",
		text: "text-red-600 dark:text-red-400",
		border: "border-red-500/40",
		soft: "bg-red-500/10",
	};
}

function getWorstScore(scores: Record<ScoreCategory, number>) {
	return SCORE_CATEGORIES.map((category) => ({
		category,
		value: scores[category] ?? 0,
	})).sort((a, b) => a.value - b.value)[0];
}

function buildNodeScoreSummary(node: INode): NodeScoreSummary | undefined {
	if (!node.scores || node.name === "reroute") return undefined;
	const worstScore = getWorstScore(node.scores);
	return {
		node,
		scores: node.scores,
		worstScore: worstScore.value,
		worstCategory: worstScore.category,
	};
}

function aggregateNodeScores(nodes: INode[]): ScoreAggregate | undefined {
	const relevantNodes = nodes.filter((node) => node.name !== "reroute");
	const scoredNodes = relevantNodes.filter(
		(node): node is INode & { scores: NodeScores } => !!node.scores,
	);

	if (scoredNodes.length === 0) return undefined;

	const scores = SCORE_CATEGORIES.reduce(
		(acc, category) => {
			acc[category] = Math.min(
				...scoredNodes.map((node) => node.scores[category] ?? 0),
			);
			return acc;
		},
		{} as Record<ScoreCategory, number>,
	);
	const worstScore = getWorstScore(scores);

	return {
		scores,
		worstScore: worstScore.value,
		worstCategory: worstScore.category,
		scoredNodeCount: scoredNodes.length,
		nodeCount: relevantNodes.length,
	};
}

// Read-only wrapped node types – pointer-events disabled on flow/comment
// nodes to prevent editing, but layer nodes keep events for double-click.
const PreviewFlowNode = memo((props: NodeProps<FlowNodeType>) => {
	const worstScore = props.data.node.scores
		? getWorstScore(props.data.node.scores)
		: undefined;

	return (
		<div className="relative">
			<div className="pointer-events-none">
				<FlowNode {...props} />
			</div>
			{worstScore && (
				<div
					className={cn(
						"pointer-events-none absolute -top-6 right-0 z-10 flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-[10px] font-semibold tabular-nums shadow-sm backdrop-blur",
						scoreTone(worstScore.value).border,
						scoreTone(worstScore.value).soft,
						scoreTone(worstScore.value).text,
					)}
				>
					<span>{SCORE_ABBR[worstScore.category]}</span>
					<span>{worstScore.value}</span>
				</div>
			)}
		</div>
	);
});
PreviewFlowNode.displayName = "PreviewFlowNode";

const PreviewLayerNode = memo((props: NodeProps<LayerNodeType>) => (
	<LayerNode {...props} />
));
PreviewLayerNode.displayName = "PreviewLayerNode";

const PreviewCallFunctionNode = memo((props: NodeProps<FlowNodeType>) => (
	<div className="pointer-events-none">
		<CallFunctionNode {...props} />
	</div>
));
PreviewCallFunctionNode.displayName = "PreviewCallFunctionNode";

const PreviewLayerInnerNode = memo((props: NodeProps<ILayerInnerNode>) => (
	<div className="pointer-events-none">
		<LayerInnerNode {...props} />
	</div>
));
PreviewLayerInnerNode.displayName = "PreviewLayerInnerNode";

const PreviewCommentNode = memo((props: NodeProps<CommentNodeType>) => (
	<div className="pointer-events-none">
		<CommentNode {...props} />
	</div>
));
PreviewCommentNode.displayName = "PreviewCommentNode";

const DATA_TYPE_LABELS: Record<string, string> = {
	Boolean: "Bool",
	Byte: "Byte",
	Date: "Date",
	Execution: "Exec",
	Float: "Float",
	Generic: "Any",
	Integer: "Int",
	PathBuf: "Path",
	String: "String",
	Struct: "Struct",
};

function formatDataType(dt: string, vt?: string) {
	const base = DATA_TYPE_LABELS[dt] ?? dt;
	if (vt && vt !== "Normal") return `${base}[]`;
	return base;
}

function VariablesPanel({
	variables,
}: { variables: Record<string, IVariable> }) {
	const vars = Object.values(variables);
	if (vars.length === 0) {
		return (
			<p className="text-xs text-muted-foreground p-2">No variables defined.</p>
		);
	}

	return (
		<div className="divide-y">
			{vars.map((v) => (
				<div key={v.id} className="px-2 py-1.5">
					<div className="flex items-center gap-2">
						<span className="text-xs font-medium truncate flex-1">
							{v.name}
						</span>
						<Badge
							variant="outline"
							className="text-[10px] px-1.5 py-0 shrink-0"
						>
							{formatDataType(v.data_type, v.value_type)}
						</Badge>
					</div>
					<div className="flex items-center gap-2 mt-0.5">
						{v.exposed && (
							<Badge variant="secondary" className="text-[9px] px-1 py-0">
								exposed
							</Badge>
						)}
						{v.secret && (
							<Badge variant="destructive" className="text-[9px] px-1 py-0">
								secret
							</Badge>
						)}
						{v.editable && (
							<Badge variant="secondary" className="text-[9px] px-1 py-0">
								editable
							</Badge>
						)}
						{v.description && (
							<span className="text-[10px] text-muted-foreground truncate">
								{v.description}
							</span>
						)}
					</div>
				</div>
			))}
		</div>
	);
}

function LayerIcon({ type }: { type?: ILayerType | string }) {
	const label = type === "Function" ? "fn" : type === "Macro" ? "M" : "L";
	return (
		<span className="inline-flex items-center justify-center h-5 w-5 rounded bg-muted text-[10px] font-bold shrink-0">
			{label}
		</span>
	);
}

function ScorePill({
	category,
	value,
	compact = false,
}: {
	category: ScoreCategory;
	value: number;
	compact?: boolean;
}) {
	const Icon = SCORE_ICONS[category];
	const tone = scoreTone(value);
	return (
		<div
			className={cn(
				"inline-flex items-center gap-1 rounded-md border font-semibold tabular-nums",
				compact ? "px-1.5 py-0.5 text-[10px]" : "px-2 py-1 text-xs",
				tone.border,
				tone.soft,
				tone.text,
			)}
			title={`${SCORE_LABELS[category]}: ${value}/10`}
		>
			<Icon className={compact ? "h-3 w-3" : "h-3.5 w-3.5"} />
			<span>{SCORE_ABBR[category]}</span>
			<span>{value}</span>
		</div>
	);
}

function ScoreBarRow({
	category,
	value,
}: {
	category: ScoreCategory;
	value: number;
}) {
	const Icon = SCORE_ICONS[category];
	const tone = scoreTone(value);
	return (
		<div className="space-y-1">
			<div className="flex items-center gap-2 text-xs">
				<Icon className={cn("h-3.5 w-3.5 shrink-0", tone.text)} />
				<span className="flex-1 text-muted-foreground">
					{SCORE_LABELS[category]}
				</span>
				<span className={cn("font-semibold tabular-nums", tone.text)}>
					{value}/10
				</span>
			</div>
			<div className="h-1.5 overflow-hidden rounded-full bg-muted">
				<div
					className={cn("h-full rounded-full", tone.bg)}
					style={{ width: `${Math.max(0, Math.min(10, value)) * 10}%` }}
				/>
			</div>
		</div>
	);
}

function ScoreSummaryStrip({
	aggregate,
}: {
	aggregate?: ScoreAggregate;
}) {
	if (!aggregate) {
		return (
			<Badge variant="outline" className="shrink-0 text-[11px]">
				No scores
			</Badge>
		);
	}

	return (
		<div className="flex flex-wrap items-center justify-end gap-1.5">
			<ScorePill
				category={aggregate.worstCategory}
				value={aggregate.worstScore}
			/>
			<div className="hidden xl:flex flex-wrap justify-end gap-1">
				{SCORE_CATEGORIES.map((category) => (
					<ScorePill
						key={category}
						category={category}
						value={aggregate.scores[category]}
						compact
					/>
				))}
			</div>
		</div>
	);
}

function SelectedNodeInspector({
	node,
	onFocusNode,
}: {
	node?: INode;
	onFocusNode: (nodeId: string) => void;
}) {
	if (!node) {
		return (
			<div className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
				<div className="flex items-start gap-2">
					<Info className="mt-0.5 h-3.5 w-3.5 shrink-0" />
					<span>No node selected.</span>
				</div>
			</div>
		);
	}

	const summary = buildNodeScoreSummary(node);

	return (
		<div className="space-y-3 rounded-md border bg-card/60 p-3">
			<div className="flex items-start justify-between gap-2">
				<div className="min-w-0">
					<p className="truncate text-xs font-semibold">
						{node.friendly_name || node.name}
					</p>
					<p className="truncate font-mono text-[10px] text-muted-foreground">
						{node.name}
					</p>
				</div>
				<button
					type="button"
					className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
					onClick={() => onFocusNode(node.id)}
					title="Focus node"
				>
					<Crosshair className="h-3.5 w-3.5" />
				</button>
			</div>
			{summary ? (
				<div className="space-y-2.5">
					<ScorePill
						category={summary.worstCategory}
						value={summary.worstScore}
					/>
					<div className="space-y-2.5">
						{SCORE_CATEGORIES.map((category) => (
							<ScoreBarRow
								key={category}
								category={category}
								value={summary.scores[category]}
							/>
						))}
					</div>
				</div>
			) : (
				<p className="text-xs text-muted-foreground">
					This node does not define governance scores.
				</p>
			)}
		</div>
	);
}

function ScoresPanel({
	aggregate,
	worstNodes,
	selectedNode,
	onFocusNode,
}: {
	aggregate?: ScoreAggregate;
	worstNodes: NodeScoreSummary[];
	selectedNode?: INode;
	onFocusNode: (nodeId: string) => void;
}) {
	if (!aggregate) {
		return (
			<div className="space-y-3 p-3">
				<SelectedNodeInspector node={selectedNode} onFocusNode={onFocusNode} />
				<p className="text-xs text-muted-foreground">
					No scored nodes were found in this board.
				</p>
			</div>
		);
	}

	return (
		<div className="space-y-4 p-3">
			<div className="space-y-3 rounded-md border bg-card/60 p-3">
				<div className="flex items-center justify-between gap-3">
					<div>
						<p className="text-xs font-semibold">Board score</p>
						<p className="text-[10px] text-muted-foreground">
							Minimum score across scored nodes
						</p>
					</div>
					<ScorePill
						category={aggregate.worstCategory}
						value={aggregate.worstScore}
					/>
				</div>
				<div className="space-y-2.5">
					{SCORE_CATEGORIES.map((category) => (
						<ScoreBarRow
							key={category}
							category={category}
							value={aggregate.scores[category]}
						/>
					))}
				</div>
				<p className="text-[10px] text-muted-foreground">
					{aggregate.scoredNodeCount}/{aggregate.nodeCount} nodes scored
				</p>
			</div>

			<SelectedNodeInspector node={selectedNode} onFocusNode={onFocusNode} />

			<div className="space-y-2">
				<h4 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
					Worst nodes
				</h4>
				{worstNodes.length === 0 ? (
					<p className="text-xs text-muted-foreground">
						No scored nodes found.
					</p>
				) : (
					<div className="space-y-1.5">
						{worstNodes.map((item) => {
							const tone = scoreTone(item.worstScore);
							return (
								<button
									key={item.node.id}
									type="button"
									className={cn(
										"flex w-full items-center gap-2 rounded-md border px-2 py-1.5 text-left transition-colors hover:bg-muted/50",
										tone.border,
									)}
									onClick={() => onFocusNode(item.node.id)}
								>
									<div className="min-w-0 flex-1">
										<p className="truncate text-xs font-medium">
											{item.node.friendly_name || item.node.name}
										</p>
										<p className="truncate text-[10px] text-muted-foreground">
											{SCORE_LABELS[item.worstCategory]}
										</p>
									</div>
									<span
										className={cn(
											"inline-flex h-6 min-w-6 items-center justify-center rounded-md px-1.5 text-xs font-semibold tabular-nums",
											tone.bg,
											"text-white",
										)}
									>
										{item.worstScore}
									</span>
								</button>
							);
						})}
					</div>
				)}
			</div>
		</div>
	);
}

function LayersPanel({
	layers,
	layerNodeCounts,
	layerScores,
	onNavigate,
}: {
	layers: Record<string, ILayer>;
	layerNodeCounts: Record<string, number>;
	layerScores: Record<string, ScoreAggregate | undefined>;
	onNavigate: (layerId: string) => void;
}) {
	const layerList = Object.values(layers);
	if (layerList.length === 0) {
		return (
			<p className="text-xs text-muted-foreground p-2">No layers defined.</p>
		);
	}

	// Group by type
	const functions = layerList.filter((l) => l.type === "Function");
	const macros = layerList.filter((l) => l.type === "Macro");
	const collapsed = layerList.filter(
		(l) => l.type !== "Function" && l.type !== "Macro",
	);

	const renderGroup = (title: string, items: ILayer[]) => {
		if (items.length === 0) return null;
		return (
			<div className="space-y-1">
				<h4 className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider px-2 pt-2">
					{title} ({items.length})
				</h4>
				{items.map((layer) => (
					<button
						key={layer.id}
						type="button"
						className="flex items-center gap-2 w-full px-2 py-1.5 text-left hover:bg-muted/50 rounded-sm transition-colors"
						onClick={() => onNavigate(layer.id)}
					>
						<LayerIcon type={layer.type} />
						<span className="text-xs font-medium truncate flex-1">
							{layer.name || layer.id}
						</span>
						{layerScores[layer.id] && (
							<span
								className={cn(
									"inline-flex h-5 min-w-5 items-center justify-center rounded px-1 text-[10px] font-semibold tabular-nums text-white",
									scoreTone(layerScores[layer.id]?.worstScore ?? 0).bg,
								)}
								title={`Worst layer score: ${layerScores[layer.id]?.worstScore}/10`}
							>
								{layerScores[layer.id]?.worstScore}
							</span>
						)}
						<span className="text-[10px] text-muted-foreground">
							{Math.max(
								Object.keys(layer.nodes).length,
								layerNodeCounts[layer.id] ?? 0,
							)}{" "}
							nodes
						</span>
						<ChevronRight className="h-3 w-3 text-muted-foreground shrink-0" />
					</button>
				))}
			</div>
		);
	};

	return (
		<div className="space-y-1">
			{renderGroup("Functions", functions)}
			{renderGroup("Macros", macros)}
			{renderGroup("Layers", collapsed)}
		</div>
	);
}

function SidebarSection({
	icon: Icon,
	title,
	count,
	children,
}: {
	icon: LucideIcon;
	title: string;
	count: number;
	children: React.ReactNode;
}) {
	return (
		<div>
			<div className="flex items-center gap-1.5 px-2 py-1.5 text-xs font-medium text-muted-foreground">
				<Icon className="h-3.5 w-3.5" />
				{title} ({count})
			</div>
			<Separator />
			{children}
		</div>
	);
}

function BoardPreviewInner({ board }: { board: IBoard }) {
	const { resolvedTheme } = useTheme();
	const instanceRef = useRef<ReactFlowInstance | null>(null);
	const boardRef = useRef<IBoard | undefined>(board);
	boardRef.current = board;
	const pendingFocusNodeIdRef = useRef<string | undefined>(undefined);
	const [currentLayer, setCurrentLayer] = useState<string | undefined>();
	const [layerPath, setLayerPath] = useState<string | undefined>();
	const [selectedNodeId, setSelectedNodeId] = useState<string | undefined>();

	const colorMode = useMemo(
		() => (resolvedTheme === "dark" ? "dark" : "light"),
		[resolvedTheme],
	);

	const allNodes = useMemo(() => Object.values(board.nodes), [board.nodes]);
	const visibleNodes = useMemo(
		() =>
			allNodes.filter((node) => normalizedLayerId(node.layer) === currentLayer),
		[allNodes, currentLayer],
	);
	const boardScoreAggregate = useMemo(
		() => aggregateNodeScores(allNodes),
		[allNodes],
	);
	const currentLayerScoreAggregate = useMemo(
		() => aggregateNodeScores(visibleNodes),
		[visibleNodes],
	);
	const activeScoreAggregate = currentLayer
		? currentLayerScoreAggregate
		: boardScoreAggregate;
	const selectedNode = selectedNodeId ? board.nodes[selectedNodeId] : undefined;
	const selectedNodeSet = useMemo(
		() => new Set(selectedNodeId ? [selectedNodeId] : []),
		[selectedNodeId],
	);
	const worstNodes = useMemo(
		() =>
			allNodes
				.map(buildNodeScoreSummary)
				.filter((item): item is NodeScoreSummary => !!item)
				.sort(
					(a, b) =>
						a.worstScore - b.worstScore ||
						a.node.friendly_name.localeCompare(b.node.friendly_name) ||
						a.node.name.localeCompare(b.node.name),
				)
				.slice(0, 12),
		[allNodes],
	);
	const layerNodeCounts = useMemo(() => {
		const counts: Record<string, number> = {};
		for (const node of allNodes) {
			const layerId = normalizedLayerId(node.layer);
			if (!layerId || node.name === "reroute") continue;
			counts[layerId] = (counts[layerId] ?? 0) + 1;
		}
		return counts;
	}, [allNodes]);
	const layerScores = useMemo(() => {
		const grouped: Record<string, INode[]> = {};
		for (const node of allNodes) {
			const layerId = normalizedLayerId(node.layer);
			if (!layerId) continue;
			grouped[layerId] ??= [];
			grouped[layerId].push(node);
		}

		return Object.fromEntries(
			Object.entries(grouped).map(([layerId, nodes]) => [
				layerId,
				aggregateNodeScores(nodes),
			]),
		) as Record<string, ScoreAggregate | undefined>;
	}, [allNodes]);

	const nodeTypes = useMemo(
		() => ({
			flowNode: PreviewFlowNode,
			commentNode: PreviewCommentNode,
			layerNode: PreviewLayerNode,
			layerInnerNode: PreviewLayerInnerNode,
			callFunctionNode: PreviewCallFunctionNode,
			node: PreviewFlowNode,
		}),
		[],
	);

	const pushLayer = useCallback((layer: ILayer) => {
		setSelectedNodeId(undefined);
		setCurrentLayer(layer.id);
		setLayerPath((prev) => (prev ? `${prev}/${layer.id}` : layer.id));
	}, []);

	const handleBreadcrumbNav = useCallback((path?: string) => {
		setSelectedNodeId(undefined);
		if (!path) {
			setCurrentLayer(undefined);
			setLayerPath(undefined);
		} else {
			const segments = path.split("/");
			setCurrentLayer(segments[segments.length - 1]);
			setLayerPath(path);
		}
	}, []);

	const focusRenderedNode = useCallback((nodeId: string, attempt = 0) => {
		const instance = instanceRef.current;
		if (!instance) return;

		if (instance.getNodes().some((node) => node.id === nodeId)) {
			pendingFocusNodeIdRef.current = undefined;
			instance.fitView({
				nodes: [{ id: nodeId }],
				padding: 0.45,
				duration: 450,
				maxZoom: 1.2,
			});
			return;
		}

		if (attempt >= 12) {
			pendingFocusNodeIdRef.current = undefined;
			return;
		}

		requestAnimationFrame(() => focusRenderedNode(nodeId, attempt + 1));
	}, []);

	const focusNode = useCallback(
		(nodeId: string) => {
			const node = board.nodes[nodeId];
			if (!node) return;

			setSelectedNodeId(nodeId);
			pendingFocusNodeIdRef.current = nodeId;

			const layerTree: string[] = [];
			let parentLayer = normalizedLayerId(node.layer);
			let iteration = 0;
			while (parentLayer && iteration < 40) {
				iteration++;
				const layer = board.layers[parentLayer];
				if (!layer) break;
				layerTree.push(layer.id);
				parentLayer = normalizedLayerId(layer.parent_id);
			}

			if (layerTree.length > 0) {
				setCurrentLayer(layerTree[0]);
				setLayerPath(layerTree.slice().reverse().join("/"));
			} else {
				setCurrentLayer(undefined);
				setLayerPath(undefined);
			}

			requestAnimationFrame(() => focusRenderedNode(nodeId));
		},
		[board.layers, board.nodes, focusRenderedNode],
	);

	const navigateToLayer = useCallback(
		(layerId: string) => {
			setSelectedNodeId(undefined);
			// Build path through parent chain
			const buildPath = (id: string): string => {
				const layer = board.layers[id];
				if (!layer?.parent_id || !board.layers[layer.parent_id]) return id;
				return `${buildPath(layer.parent_id)}/${id}`;
			};
			const path = buildPath(layerId);
			setLayerPath(path);
			setCurrentLayer(layerId);
		},
		[board.layers],
	);

	const { boardNodes, edges } = useMemo(() => {
		const parsed = parseBoard(
			board,
			"",
			async () => {},
			pushLayer,
			async () => {},
			async () => {},
			selectedNodeSet,
			undefined,
			undefined,
			undefined,
			currentLayer,
			boardRef,
		);
		return { boardNodes: parsed.nodes, edges: parsed.edges };
	}, [board, currentLayer, pushLayer, selectedNodeSet]);

	const handleInit = useCallback((instance: ReactFlowInstance) => {
		instanceRef.current = instance;
		instance.fitView({ padding: 0.3 });
	}, []);

	const onNodeClick = useCallback(
		(_event: React.MouseEvent, node: ReactFlowNode) => {
			const nodeData = node.data as { node?: INode };
			const nodeId = nodeData.node?.id;
			if (nodeId) setSelectedNodeId(nodeId);
		},
		[],
	);

	const onNodeDoubleClick = useCallback(
		(_event: React.MouseEvent, node: ReactFlowNode) => {
			if (node?.type === "layerNode") {
				const layerData = node.data as { layer?: ILayer };
				if (layerData.layer) pushLayer(layerData.layer);
			}
		},
		[pushLayer],
	);

	// Re-fit on layer change
	// biome-ignore lint/correctness/useExhaustiveDependencies: need to re-fit when layer changes
	useEffect(() => {
		const timer = setTimeout(() => {
			const pendingNodeId = pendingFocusNodeIdRef.current;
			if (pendingNodeId) {
				focusRenderedNode(pendingNodeId);
				return;
			}
			instanceRef.current?.fitView({ padding: 0.25 });
		}, 100);
		return () => clearTimeout(timer);
	}, [currentLayer, focusRenderedNode]);

	// Current variables: board-level or layer-level
	const currentVariables = useMemo(() => {
		if (currentLayer && board.layers[currentLayer]) {
			return board.layers[currentLayer].variables;
		}
		return board.variables;
	}, [board, currentLayer]);

	const currentTitle = currentLayer
		? board.layers[currentLayer]?.name ||
			board.layers[currentLayer]?.id ||
			currentLayer
		: board.name || "Untitled board";
	const currentNodeCount = visibleNodes.filter(
		(node) => node.name !== "reroute",
	).length;

	return (
		<div className="flex h-full min-h-0 w-full overflow-hidden bg-background">
			{/* Main flow view */}
			<div className="flex min-w-0 flex-1 flex-col">
				<div className="shrink-0 border-b bg-background/95 px-3 py-2">
					<div className="flex items-start justify-between gap-3">
						<div className="min-w-0">
							<p className="truncate text-sm font-semibold">{currentTitle}</p>
							<p className="mt-0.5 text-xs text-muted-foreground">
								{currentLayer
									? `${currentNodeCount} nodes in layer`
									: `${Object.keys(board.nodes).length} nodes`}{" "}
								· {Object.keys(board.layers).length} layers ·{" "}
								{Object.keys(currentVariables).length} variables
							</p>
						</div>
						<ScoreSummaryStrip aggregate={activeScoreAggregate} />
					</div>
					<div className="mt-2">
						<FlowBreadCrumb
							currentPath={layerPath}
							layers={board.layers}
							onAdjustPath={handleBreadcrumbNav}
						/>
					</div>
				</div>

				{/* ReactFlow */}
				<div className="min-h-0 flex-1">
					<ReactFlow
						suppressHydrationWarning
						className="w-full h-full"
						colorMode={colorMode}
						elementsSelectable
						nodesDraggable={false}
						nodesConnectable={false}
						panOnDrag
						zoomOnScroll
						zoomOnPinch
						zoomOnDoubleClick={false}
						nodes={boardNodes}
						nodeTypes={nodeTypes}
						edges={edges}
						onInit={handleInit}
						onNodeClick={onNodeClick}
						onNodeDoubleClick={onNodeDoubleClick}
						fitView
						fitViewOptions={{ padding: 0.25 }}
						proOptions={{ hideAttribution: true }}
					>
						<Background
							variant={
								currentLayer ? BackgroundVariant.Lines : BackgroundVariant.Dots
							}
							color={
								currentLayer
									? "color-mix(in oklch, var(--foreground) 5%, transparent)"
									: "color-mix(in oklch, var(--foreground) 20%, transparent)"
							}
							bgColor="color-mix(in oklch, var(--background) 80%, transparent)"
							gap={12}
							size={1}
						/>
					</ReactFlow>
				</div>
			</div>

			{/* Sidebar */}
			<div className="flex w-80 shrink-0 flex-col border-l bg-background xl:w-96">
				<Tabs defaultValue="layers" className="flex flex-col h-full">
					<TabsList className="grid w-full grid-cols-3 shrink-0 rounded-none border-b">
						<TabsTrigger value="layers" className="text-xs">
							<Layers className="h-3 w-3 mr-1" />
							Layers
						</TabsTrigger>
						<TabsTrigger value="scores" className="text-xs">
							<ShieldIcon className="h-3 w-3 mr-1" />
							Scores
						</TabsTrigger>
						<TabsTrigger value="variables" className="text-xs">
							<Variable className="h-3 w-3 mr-1" />
							Variables
						</TabsTrigger>
					</TabsList>
					<ScrollArea className="min-h-0 flex-1">
						<TabsContent value="layers" className="mt-0">
							<LayersPanel
								layers={board.layers}
								layerNodeCounts={layerNodeCounts}
								layerScores={layerScores}
								onNavigate={navigateToLayer}
							/>
						</TabsContent>
						<TabsContent value="scores" className="mt-0">
							<ScoresPanel
								aggregate={boardScoreAggregate}
								worstNodes={worstNodes}
								selectedNode={selectedNode}
								onFocusNode={focusNode}
							/>
						</TabsContent>
						<TabsContent value="variables" className="mt-0">
							<SidebarSection
								icon={Variable}
								title="Variables"
								count={Object.keys(currentVariables).length}
							>
								<VariablesPanel variables={currentVariables} />
							</SidebarSection>
						</TabsContent>
					</ScrollArea>
				</Tabs>
			</div>
		</div>
	);
}

export interface AdminBoardPreviewProps {
	board: IBoard;
}

export function AdminBoardPreview({ board }: AdminBoardPreviewProps) {
	const hasNodes = Object.keys(board.nodes).length > 0;

	if (!hasNodes && Object.keys(board.layers).length === 0) {
		return (
			<div className="flex items-center justify-center h-full">
				<p className="text-sm text-muted-foreground">This board is empty.</p>
			</div>
		);
	}

	return (
		<ReactFlowProvider>
			<BoardPreviewInner board={board} />
		</ReactFlowProvider>
	);
}
