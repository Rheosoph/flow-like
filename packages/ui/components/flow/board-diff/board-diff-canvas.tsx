"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Background,
	BackgroundVariant,
	BaseEdge,
	type EdgeProps,
	MiniMap,
	type NodeProps,
	ReactFlow,
	type Edge as ReactFlowEdge,
	type ReactFlowInstance,
	type Node as ReactFlowNode,
	ReactFlowProvider,
	type Viewport,
	getBezierPath,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useTheme } from "next-themes";
import {
	type ReactNode,
	createContext,
	memo,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
} from "react";
import { resolveLayerChain } from "../../../hooks/use-layer-navigation";
import type {
	BoardChangeStatus,
	BoardEdgeStatus,
	IBoardChange,
} from "../../../lib/board-diff";
import { parseBoard } from "../../../lib/flow-board-utils";
import type { IBoard, ILayer } from "../../../lib/schema/flow/board";
import { cn } from "../../../lib/utils";
import { CallFunctionNode } from "../call-function-node";
import {
	CommentNode,
	type CommentNode as CommentNodeType,
} from "../comment-node";
import { FlowBreadCrumb } from "../flow-breadcrumb";
import { FlowNode, type FlowNode as FlowNodeType } from "../flow-node";
import { type ILayerInnerNode, LayerInnerNode } from "../layer-inner-node";
import { LayerNode, type LayerNode as LayerNodeType } from "../layer-node";
import { MediaNode, type MediaNode as MediaNodeType } from "../media-node";
import { STATUS_TONE, useDetailLabel, useStatusLabel } from "./diff-status";

interface DiffCanvasState {
	statuses: Map<string, BoardChangeStatus>;
	inner: Set<string>;
	changes: Map<string, IBoardChange>;
	selected: Set<string>;
	dim: boolean;
	detailSide: "both" | "before" | "after";
}

const DiffCanvasContext = createContext<DiffCanvasState>({
	statuses: new Map(),
	inner: new Set(),
	changes: new Map(),
	selected: new Set(),
	dim: false,
	detailSide: "both",
});

const MAX_CARD_ROWS = 6;

function DetailCard({ change }: Readonly<{ change: IBoardChange }>) {
	const { detailSide } = useContext(DiffCanvasContext);
	const detailLabel = useDetailLabel();
	const rows = change.details.slice(0, MAX_CARD_ROWS);
	if (!rows.length) return null;
	return (
		<div className="pointer-events-none absolute left-0 top-full z-50 mt-3 w-72 rounded-md border bg-popover p-2.5 text-popover-foreground shadow-lg">
			<div className="grid gap-1.5">
				{rows.map((detail) => (
					<div key={detail.field} className="grid gap-0.5 text-[11px]">
						<span className="font-medium text-muted-foreground">
							{detailLabel(detail)}
						</span>
						{detailSide !== "after" && detail.before !== undefined && (
							<span className="truncate rounded-sm bg-destructive/10 px-1 font-mono line-through decoration-destructive/50">
								{detail.before}
							</span>
						)}
						{detailSide !== "before" && detail.after !== undefined && (
							<span className="truncate rounded-sm bg-emerald-500/10 px-1 font-mono">
								{detail.after}
							</span>
						)}
					</div>
				))}
			</div>
		</div>
	);
}

function DiffFrame({
	id,
	children,
	rounded = "rounded-md",
}: Readonly<{ id: string; children: ReactNode; rounded?: string }>) {
	const { t } = useTranslation("flow");
	const statusLabel = useStatusLabel();
	const { statuses, inner, changes, selected, dim } =
		useContext(DiffCanvasContext);
	const status = statuses.get(id);
	const hasInner = !status && inner.has(id);
	const isSelected = selected.has(id);
	const change = changes.get(id);
	return (
		<div
			className={cn(
				"relative transition-opacity",
				dim && !status && !hasInner && !isSelected && "opacity-40",
			)}
		>
			{(status || hasInner) && (
				<div
					aria-hidden
					className={cn(
						"pointer-events-none absolute -inset-1.5 border-2",
						rounded,
						status
							? STATUS_TONE[status].border
							: "border-dashed border-sky-500/70",
						status === "removed" && "border-dashed",
					)}
				/>
			)}
			{isSelected && (
				<div
					aria-hidden
					className={cn(
						"pointer-events-none absolute -inset-3 border-2 border-primary",
						rounded,
					)}
				/>
			)}
			<div
				className={cn(
					"pointer-events-none",
					status === "removed" && "opacity-60 saturate-50",
				)}
			>
				{children}
			</div>
			{(status || hasInner) && (
				<span
					className={cn(
						"pointer-events-none absolute -top-3 right-2 z-10 rounded-full px-2 py-0.5 text-[10px] font-semibold leading-none shadow-sm",
						status
							? STATUS_TONE[status].solid
							: "bg-sky-500/15 text-sky-700 dark:text-sky-300",
					)}
				>
					{status
						? statusLabel(status)
						: t("boardDiffChangesInside", "Changes inside")}
				</span>
			)}
			{isSelected && change && change.status === "changed" && (
				<DetailCard change={change} />
			)}
		</div>
	);
}

const DiffFlowNode = memo((props: NodeProps<FlowNodeType>) => (
	<DiffFrame id={props.id}>
		<FlowNode {...props} />
	</DiffFrame>
));
DiffFlowNode.displayName = "DiffFlowNode";

const DiffCallFunctionNode = memo((props: NodeProps<FlowNodeType>) => (
	<DiffFrame id={props.id}>
		<CallFunctionNode {...props} />
	</DiffFrame>
));
DiffCallFunctionNode.displayName = "DiffCallFunctionNode";

const DiffLayerNode = memo((props: NodeProps<LayerNodeType>) => (
	<DiffFrame id={props.id}>
		<LayerNode {...props} />
	</DiffFrame>
));
DiffLayerNode.displayName = "DiffLayerNode";

const DiffLayerInnerNode = memo((props: NodeProps<ILayerInnerNode>) => (
	<div className="pointer-events-none">
		<LayerInnerNode {...props} />
	</div>
));
DiffLayerInnerNode.displayName = "DiffLayerInnerNode";

const DiffCommentNode = memo((props: NodeProps<CommentNodeType>) => (
	<DiffFrame id={props.id}>
		<CommentNode {...props} />
	</DiffFrame>
));
DiffCommentNode.displayName = "DiffCommentNode";

const DiffMediaNode = memo((props: NodeProps<MediaNodeType>) => (
	<DiffFrame id={props.id}>
		<MediaNode {...props} />
	</DiffFrame>
));
DiffMediaNode.displayName = "DiffMediaNode";

const DiffEdge = memo(function DiffEdge({
	id,
	sourceX,
	sourceY,
	targetX,
	targetY,
	sourcePosition,
	targetPosition,
	style,
	data,
}: EdgeProps) {
	const [path] = getBezierPath({
		sourceX,
		sourceY,
		targetX,
		targetY,
		sourcePosition,
		targetPosition,
	});
	const status = data?.diff as BoardEdgeStatus | undefined;
	const stroke = status
		? STATUS_TONE[status].stroke
		: ((style?.stroke as string | undefined) ?? "var(--foreground)");
	return (
		<BaseEdge
			id={id}
			path={path}
			style={{
				stroke,
				strokeWidth: status ? 2.75 : 1.5,
				strokeDasharray: status === "removed" ? "7 5" : undefined,
				opacity: status ? 1 : data?.dim ? 0.18 : 0.55,
			}}
		/>
	);
});

const NODE_TYPES = {
	node: DiffFlowNode,
	flowNode: DiffFlowNode,
	callFunctionNode: DiffCallFunctionNode,
	layerNode: DiffLayerNode,
	layerInnerNode: DiffLayerInnerNode,
	commentNode: DiffCommentNode,
	mediaNode: DiffMediaNode,
};
const EDGE_TYPES = { diff: DiffEdge };
const noop = async () => {};

export interface BoardDiffCanvasProps {
	board: IBoard;
	statuses: Map<string, BoardChangeStatus>;
	inner: Set<string>;
	edgeStatus: Map<string, BoardEdgeStatus>;
	changes: Map<string, IBoardChange>;
	detailSide: "both" | "before" | "after";
	layerId?: string;
	onLayerChange: (layerId?: string) => void;
	selectedIds: string[];
	focusRequest: number;
	onSelectId: (id: string) => void;
	dim: boolean;
	tag?: ReactNode;
	onInstance?: (instance: ReactFlowInstance | null) => void;
	onUserMove?: (viewport: Viewport) => void;
}

function Canvas({
	board,
	statuses,
	inner,
	edgeStatus,
	changes,
	detailSide,
	layerId,
	onLayerChange,
	selectedIds,
	focusRequest,
	onSelectId,
	dim,
	tag,
	onInstance,
	onUserMove,
}: Readonly<BoardDiffCanvasProps>) {
	const { resolvedTheme } = useTheme();
	const instanceRef = useRef<ReactFlowInstance | null>(null);
	const boardRef = useRef<IBoard | undefined>(board);
	boardRef.current = board;
	const selected = useMemo(() => new Set(selectedIds), [selectedIds]);

	const pushLayer = useCallback(
		(layer: ILayer) => onLayerChange(layer.id),
		[onLayerChange],
	);

	const { nodes, edges } = useMemo(() => {
		const version =
			board.version?.length === 3
				? (board.version as [number, number, number])
				: ([0, 0, 0] as [number, number, number]);
		const parsed = parseBoard(
			board,
			"",
			noop,
			pushLayer,
			noop,
			noop,
			new Set(),
			undefined,
			undefined,
			undefined,
			layerId,
			boardRef,
			version,
		);
		return {
			nodes: parsed.nodes as ReactFlowNode[],
			edges: parsed.edges.map(
				(edge: ReactFlowEdge): ReactFlowEdge => ({
					...edge,
					type: "diff",
					animated: false,
					selectable: false,
					data: { diff: edgeStatus.get(edge.id), dim },
				}),
			),
		};
	}, [board, dim, edgeStatus, layerId, pushLayer]);

	const context = useMemo<DiffCanvasState>(
		() => ({ statuses, inner, changes, selected, dim, detailSide }),
		[changes, detailSide, dim, inner, selected, statuses],
	);

	const focus = useCallback((ids: string[], attempt = 0) => {
		const instance = instanceRef.current;
		if (!instance || !ids.length) return;
		const present = ids.filter((id) => instance.getNode(id));
		if (present.length) {
			instance.fitView({
				nodes: present.map((id) => ({ id })),
				padding: 0.45,
				maxZoom: 1.1,
				duration: 400,
			});
			return;
		}
		if (attempt < 12) requestAnimationFrame(() => focus(ids, attempt + 1));
	}, []);

	const selectedKey = selectedIds.join("|");
	// biome-ignore lint/correctness/useExhaustiveDependencies: focus runs on explicit requests and layer switches, not on every selection render
	useEffect(() => {
		const timer = setTimeout(() => {
			if (selectedIds.length) focus(selectedIds);
			else instanceRef.current?.fitView({ padding: 0.2, duration: 300 });
		}, 60);
		return () => clearTimeout(timer);
	}, [focusRequest, layerId, selectedKey]);

	const layerPath = useMemo(() => {
		if (!layerId) return undefined;
		const chain = resolveLayerChain(board.layers, layerId);
		return chain.length ? chain.join("/") : layerId;
	}, [board.layers, layerId]);

	const minimapColor = useCallback(
		(node: ReactFlowNode) => {
			const status = statuses.get(node.id);
			if (status) return STATUS_TONE[status].stroke;
			if (inner.has(node.id)) return "var(--color-sky-500)";
			return "color-mix(in oklch, var(--muted-foreground) 35%, transparent)";
		},
		[inner, statuses],
	);

	return (
		<DiffCanvasContext.Provider value={context}>
			<div className="relative flex h-full min-h-0 flex-col">
				{(layerId || tag) && (
					<div className="flex min-h-9 shrink-0 items-center gap-3 border-b bg-background/95 px-3 py-1.5 text-xs">
						{tag}
						{layerId && (
							<FlowBreadCrumb
								currentPath={layerPath}
								layers={board.layers}
								onAdjustPath={(path) =>
									onLayerChange(path ? path.split("/").at(-1) : undefined)
								}
							/>
						)}
					</div>
				)}
				<div className="min-h-0 flex-1">
					<ReactFlow
						colorMode={resolvedTheme === "dark" ? "dark" : "light"}
						nodes={nodes}
						edges={edges}
						nodeTypes={NODE_TYPES}
						edgeTypes={EDGE_TYPES}
						nodesDraggable={false}
						nodesConnectable={false}
						elementsSelectable={false}
						deleteKeyCode={null}
						selectionKeyCode={null}
						multiSelectionKeyCode={null}
						zoomOnDoubleClick={false}
						minZoom={0.05}
						onInit={(instance) => {
							instanceRef.current = instance;
							onInstance?.(instance);
							instance.fitView({ padding: 0.2 });
						}}
						onMove={(event, viewport) => {
							if (event) onUserMove?.(viewport);
						}}
						onNodeClick={(_event, node) => onSelectId(node.id)}
						onNodeDoubleClick={(_event, node) => {
							if (node.type === "layerNode") onLayerChange(node.id);
						}}
						proOptions={{ hideAttribution: true }}
					>
						<Background
							variant={
								layerId ? BackgroundVariant.Lines : BackgroundVariant.Dots
							}
							color={`color-mix(in oklch, var(--foreground) ${layerId ? 5 : 20}%, transparent)`}
							bgColor="color-mix(in oklch, var(--background) 80%, transparent)"
							gap={12}
							size={1}
						/>
						<MiniMap
							pannable
							zoomable
							nodeColor={minimapColor}
							nodeStrokeWidth={0}
							bgColor="var(--card)"
							style={{ width: 168, height: 104 }}
							className="border! border-border! rounded-md! overflow-hidden"
							maskColor="color-mix(in oklch, var(--background) 55%, transparent)"
						/>
					</ReactFlow>
				</div>
			</div>
		</DiffCanvasContext.Provider>
	);
}

export function BoardDiffCanvas(props: Readonly<BoardDiffCanvasProps>) {
	return (
		<ReactFlowProvider>
			<Canvas {...props} />
		</ReactFlowProvider>
	);
}

const APPROX_WIDTH = 260;
const APPROX_HEIGHT = 170;

function crowded(x: number, y: number, occupied: [number, number][]) {
	return occupied.some(
		([ox, oy]) =>
			Math.abs(ox - x) < APPROX_WIDTH && Math.abs(oy - y) < APPROX_HEIGHT,
	);
}

/**
 * The head board with everything the base had and the head dropped put back in, so one
 * canvas can show both. Removed nodes that would sit on top of their replacement are
 * lifted above it; removed wires are re-attached to their source pins.
 */
export function overlayBoard(
	base: IBoard,
	head: IBoard,
	edgeStatus: Map<string, BoardEdgeStatus>,
): IBoard {
	const nodes = { ...head.nodes };
	const layers = { ...head.layers };
	const comments = { ...head.comments };
	const occupiedByLayer = new Map<string, [number, number][]>();
	for (const node of Object.values(head.nodes ?? {})) {
		const key = node.layer ?? "";
		const list = occupiedByLayer.get(key) ?? [];
		list.push([node.coordinates?.[0] ?? 0, node.coordinates?.[1] ?? 0]);
		occupiedByLayer.set(key, list);
	}

	for (const [id, node] of Object.entries(base.nodes ?? {})) {
		if (head.nodes?.[id]) continue;
		const occupied = occupiedByLayer.get(node.layer ?? "") ?? [];
		const x = node.coordinates?.[0] ?? 0;
		let y = node.coordinates?.[1] ?? 0;
		for (let step = 0; step < 6 && crowded(x, y, occupied); step++)
			y -= APPROX_HEIGHT;
		occupied.push([x, y]);
		occupiedByLayer.set(node.layer ?? "", occupied);
		nodes[id] = {
			...node,
			coordinates: [x, y, ...(node.coordinates?.slice(2) ?? [])],
		};
	}
	for (const [id, layer] of Object.entries(base.layers ?? {}))
		if (!head.layers?.[id]) layers[id] = layer;
	for (const [id, comment] of Object.entries(base.comments ?? {}))
		if (!head.comments?.[id]) comments[id] = comment;

	const owners = new Map<string, { kind: "node" | "layer"; id: string }>();
	for (const node of Object.values(nodes))
		for (const pinId of Object.keys(node.pins ?? {}))
			owners.set(pinId, { kind: "node", id: node.id });
	for (const layer of Object.values(layers))
		for (const pinId of Object.keys(layer.pins ?? {}))
			owners.set(pinId, { kind: "layer", id: layer.id });

	const reattach = (pinId: string, target: string) => {
		const owner = owners.get(pinId);
		if (!owner) return;
		const holder = owner.kind === "node" ? nodes[owner.id] : layers[owner.id];
		const pin = holder?.pins?.[pinId];
		if (!pin || pin.connected_to.includes(target)) return;
		const next = {
			...holder,
			pins: {
				...holder.pins,
				[pinId]: { ...pin, connected_to: [...pin.connected_to, target] },
			},
		};
		if (owner.kind === "node") nodes[owner.id] = next as (typeof nodes)[string];
		else layers[owner.id] = next as (typeof layers)[string];
	};
	const baseHolders = [
		...Object.values(base.nodes ?? {}),
		...Object.values(base.layers ?? {}),
	];
	for (const holder of baseHolders) {
		for (const pin of Object.values(holder.pins ?? {})) {
			for (const target of pin.connected_to ?? []) {
				if (
					edgeStatus.get(`${pin.id}-${target}`) === "removed" &&
					owners.has(target)
				)
					reattach(pin.id, target);
			}
		}
	}

	return { ...head, nodes, layers, comments };
}
