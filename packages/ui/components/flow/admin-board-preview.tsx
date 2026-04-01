"use client";

import {
	Background,
	BackgroundVariant,
	type NodeProps,
	ReactFlow,
	type ReactFlowInstance,
	ReactFlowProvider,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { ChevronRight, Layers, type LucideIcon, Variable } from "lucide-react";
import { useTheme } from "next-themes";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type IBoard, type IVariable, parseBoard } from "../../lib";
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
import {
	CommentNode,
	type CommentNode as CommentNodeType,
} from "./comment-node";
import { FlowBreadCrumb } from "./flow-breadcrumb";
import { FlowNode, type FlowNode as FlowNodeType } from "./flow-node";
import { LayerNode, type LayerNode as LayerNodeType } from "./layer-node";

// Read-only wrapped node types – pointer-events disabled on flow/comment
// nodes to prevent editing, but layer nodes keep events for double-click.
const PreviewFlowNode = memo((props: NodeProps<FlowNodeType>) => (
	<div className="pointer-events-none">
		<FlowNode {...props} />
	</div>
));
PreviewFlowNode.displayName = "PreviewFlowNode";

const PreviewLayerNode = memo((props: NodeProps<LayerNodeType>) => (
	<LayerNode {...props} />
));
PreviewLayerNode.displayName = "PreviewLayerNode";

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

function LayersPanel({
	layers,
	onNavigate,
}: {
	layers: Record<string, ILayer>;
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
						<span className="text-[10px] text-muted-foreground">
							{Object.keys(layer.nodes).length} nodes
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
	const [currentLayer, setCurrentLayer] = useState<string | undefined>();
	const [layerPath, setLayerPath] = useState<string | undefined>();

	const colorMode = useMemo(
		() => (resolvedTheme === "dark" ? "dark" : "light"),
		[resolvedTheme],
	);

	const nodeTypes = useMemo(
		() => ({
			flowNode: PreviewFlowNode,
			commentNode: PreviewCommentNode,
			layerNode: PreviewLayerNode,
			node: PreviewFlowNode,
		}),
		[],
	);

	const pushLayer = useCallback((layer: ILayer) => {
		setCurrentLayer(layer.id);
		setLayerPath((prev) => (prev ? `${prev}/${layer.id}` : layer.id));
	}, []);

	const handleBreadcrumbNav = useCallback((path?: string) => {
		if (!path) {
			setCurrentLayer(undefined);
			setLayerPath(undefined);
		} else {
			const segments = path.split("/");
			setCurrentLayer(segments[segments.length - 1]);
			setLayerPath(path);
		}
	}, []);

	const navigateToLayer = useCallback(
		(layerId: string) => {
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
			new Set(),
			undefined,
			undefined,
			undefined,
			currentLayer,
		);
		return { boardNodes: parsed.nodes, edges: parsed.edges };
	}, [board, currentLayer, pushLayer]);

	const handleInit = useCallback((instance: ReactFlowInstance) => {
		instanceRef.current = instance;
		instance.fitView({ padding: 0.3 });
	}, []);

	const onNodeDoubleClick = useCallback(
		(_event: React.MouseEvent, node: any) => {
			if (node?.type === "layerNode") {
				const layer: ILayer = node.data.layer;
				pushLayer(layer);
			}
		},
		[pushLayer],
	);

	// Re-fit on layer change
	// biome-ignore lint/correctness/useExhaustiveDependencies: need to re-fit when layer changes
	useEffect(() => {
		const timer = setTimeout(
			() => instanceRef.current?.fitView({ padding: 0.3 }),
			100,
		);
		return () => clearTimeout(timer);
	}, [currentLayer]);

	// Current variables: board-level or layer-level
	const currentVariables = useMemo(() => {
		if (currentLayer && board.layers[currentLayer]) {
			return board.layers[currentLayer].variables;
		}
		return board.variables;
	}, [board, currentLayer]);

	return (
		<div className="flex h-full">
			{/* Main flow view */}
			<div className="flex-1 flex flex-col min-w-0">
				{/* Breadcrumb */}
				<div className="border-b px-2 py-1 shrink-0">
					<FlowBreadCrumb
						currentPath={layerPath}
						layers={board.layers}
						onAdjustPath={handleBreadcrumbNav}
					/>
				</div>

				{/* ReactFlow */}
				<div className="flex-1">
					<ReactFlow
						suppressHydrationWarning
						className="w-full h-full"
						colorMode={colorMode}
						elementsSelectable={false}
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
						onNodeDoubleClick={onNodeDoubleClick}
						fitView
						fitViewOptions={{ padding: 0.3 }}
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
			<div className="w-64 border-l shrink-0 flex flex-col bg-background">
				<Tabs defaultValue="layers" className="flex flex-col h-full">
					<TabsList className="grid w-full grid-cols-2 shrink-0 rounded-none border-b">
						<TabsTrigger value="layers" className="text-xs">
							<Layers className="h-3 w-3 mr-1" />
							Layers
						</TabsTrigger>
						<TabsTrigger value="variables" className="text-xs">
							<Variable className="h-3 w-3 mr-1" />
							Variables
						</TabsTrigger>
					</TabsList>
					<ScrollArea className="flex-1">
						<TabsContent value="layers" className="mt-0">
							<LayersPanel layers={board.layers} onNavigate={navigateToLayer} />
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
