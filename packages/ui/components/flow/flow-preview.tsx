import {
	Background,
	BackgroundVariant,
	type ColorMode,
	type NodeProps,
	ReactFlow,
	type ReactFlowInstance,
	ReactFlowProvider,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useTheme } from "next-themes";
import { memo, useCallback, useEffect, useMemo, useRef } from "react";
import {
	type IBoard,
	type IComment,
	IExecutionMode,
	IExecutionStage,
	ILogLevel,
	type INode,
	parseBoard,
} from "../../lib";
import type { ILayer, IVariable } from "../../lib/schema/flow/board";
import { CallFunctionNode } from "./call-function-node";
import {
	CommentNode,
	type CommentNode as CommentNodeType,
} from "./comment-node";
import { FlowNode, type FlowNode as FlowNodeType } from "./flow-node";
import { type ILayerInnerNode, LayerInnerNode } from "./layer-inner-node";
import { LayerNode, type LayerNode as LayerNodeType } from "./layer-node";

interface FlowPreviewProps {
	nodes: INode[];
	comments?: { [key: string]: IComment };
	layers?: { [key: string]: ILayer };
	variables?: { [key: string]: IVariable };
	/** Explicit color mode for embeds that do not mount a next-themes provider. */
	colorMode?: ColorMode;
}

// Preview versions of nodes that don't show toolbars
const PreviewFlowNode = memo((props: NodeProps<FlowNodeType>) => (
	<div className="pointer-events-none">
		<FlowNode {...props} />
	</div>
));
PreviewFlowNode.displayName = "PreviewFlowNode";

const PreviewLayerNode = memo((props: NodeProps<LayerNodeType>) => (
	<div className="pointer-events-none">
		<LayerNode {...props} />
	</div>
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

function FlowPreviewInner({
	nodes,
	comments,
	layers,
	variables,
	colorMode: colorModeOverride,
}: Readonly<FlowPreviewProps>) {
	const { resolvedTheme } = useTheme();
	const instanceRef = useRef<ReactFlowInstance | null>(null);
	const boardRef = useRef<IBoard | undefined>(undefined);
	const colorMode = useMemo<ColorMode>(
		() => colorModeOverride ?? (resolvedTheme === "dark" ? "dark" : "light"),
		[colorModeOverride, resolvedTheme],
	);
	const layoutRevision = useMemo(
		() =>
			nodes
				.map(
					(node) =>
						`${node.id}:${node.hash ?? ""}:${node.coordinates?.join(",") ?? ""}`,
				)
				.join("|"),
		[nodes],
	);

	const handleInit = useCallback((instance: ReactFlowInstance) => {
		instanceRef.current = instance;
		// Initial fit
		instance.fitView({ padding: 0.3 });
	}, []);

	// Re-fit view after mount and when nodes change (handles container resize)
	useEffect(() => {
		void layoutRevision;
		const timers = [
			setTimeout(() => instanceRef.current?.fitView({ padding: 0.3 }), 50),
			setTimeout(() => instanceRef.current?.fitView({ padding: 0.3 }), 150),
			setTimeout(() => instanceRef.current?.fitView({ padding: 0.3 }), 300),
		];
		return () => {
			for (const timer of timers) clearTimeout(timer);
		};
	}, [layoutRevision]);

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

	const { boardNodes, edges } = useMemo(() => {
		const parsed: { [key: string]: INode } = {};
		for (const node of nodes) {
			parsed[node.id] = node;
		}

		const board: IBoard = {
			comments: comments ?? {},
			created_at: { nanos_since_epoch: 0, secs_since_epoch: 0 },
			description: "",
			id: "",
			log_level: ILogLevel.Info,
			name: "",
			nodes: parsed,
			refs: {},
			stage: IExecutionStage.Dev,
			updated_at: { nanos_since_epoch: 0, secs_since_epoch: 0 },
			layers: layers ?? {},
			version: [0, 0, 0],
			variables: variables ?? {},
			viewport: [0, 0, 0, 0],
			page_ids: [],
			execution_mode: IExecutionMode.Hybrid,
		};

		boardRef.current = board;

		const parsedBoard = parseBoard(
			board,
			"",
			async () => {},
			async () => {},
			async () => {},
			async () => {},
			new Set(),
			undefined,
			undefined,
			undefined,
			undefined,
			boardRef,
		);

		return { boardNodes: parsedBoard.nodes, edges: parsedBoard.edges };
	}, [nodes, comments, layers, variables]);

	return (
		<ReactFlow
			suppressHydrationWarning
			className="w-full h-full min-h-56 rounded-lg"
			colorMode={colorMode}
			elementsSelectable={false}
			nodesDraggable={false}
			nodesConnectable={false}
			panOnDrag={true}
			zoomOnScroll={true}
			zoomOnPinch={true}
			zoomOnDoubleClick={false}
			nodes={boardNodes}
			nodeTypes={nodeTypes}
			onInit={handleInit}
			fitView
			fitViewOptions={{ padding: 0.3 }}
			edges={edges}
			proOptions={{ hideAttribution: true }}
		>
			<Background variant={BackgroundVariant.Dots} gap={12} size={1} />
		</ReactFlow>
	);
}

export function FlowPreview({
	nodes,
	comments,
	layers,
	variables,
	colorMode,
}: Readonly<FlowPreviewProps>) {
	if (!nodes || nodes.length === 0) {
		return (
			<div className="w-full h-full min-h-56 rounded-md flow-preview not-content flex items-center justify-center bg-muted/20">
				<p className="text-sm text-muted-foreground">No nodes to preview</p>
			</div>
		);
	}

	return (
		<main className="w-full h-full min-h-56 rounded-md flow-preview not-content">
			<ReactFlowProvider>
				<FlowPreviewInner
					nodes={nodes}
					comments={comments}
					layers={layers}
					variables={variables}
					colorMode={colorMode}
				/>
			</ReactFlowProvider>
		</main>
	);
}
