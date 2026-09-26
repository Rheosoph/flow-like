"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Background,
	BackgroundVariant,
	BaseEdge,
	type ColorMode,
	type Connection,
	type Edge,
	EdgeLabelRenderer,
	type EdgeProps,
	Handle,
	type InternalNode,
	type Node,
	type NodeProps,
	Panel,
	Position,
	ReactFlow,
	ReactFlowProvider,
	type XYPosition,
	useConnection,
	useInternalNode,
	useNodes,
	useNodesState,
	useReactFlow,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import {
	ExternalLink,
	GitBranch,
	KeyRound,
	LayoutGrid,
	Link2,
	Maximize2,
	Minus,
	Plus,
	Scan,
	TriangleAlert,
} from "lucide-react";
import { useTheme } from "next-themes";
import {
	type CSSProperties,
	Fragment,
	createContext,
	memo,
	useCallback,
	useContext,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { cn } from "../../../lib/utils";
import type {
	EdgeLabelMapping,
	NodeLabelMapping,
} from "../../../state/backend-state/graph-state";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { getGraphIcon } from "../../ui/graph/icons";
import { layoutSchema } from "./ontology-schema-layout";
import {
	SCHEMA_NODE_WIDTH,
	type SchemaObject,
	type SchemaRect,
	type SchemaRelationship,
	type SchemaRow,
	buildSchemaModel,
	loopGeometry,
	routedCurveGeometry,
} from "./ontology-schema-model";

export interface OntologySchemaGraphProps {
	nodes: readonly NodeLabelMapping[];
	edges: readonly EdgeLabelMapping[];
	/** Display names for children in other ontologies, keyed by `externalTargetKey`. */
	externalTargets?: ReadonlyMap<string, string>;
	title?: string;
	className?: string;
	onSelectObject?: (object: NodeLabelMapping) => void;
	onSelectRelationship?: (index: number) => void;
	/** Enables drag-to-link; receives the two objects the user connected. */
	onConnect?: (source: NodeLabelMapping, target: NodeLabelMapping) => void;
}

type SchemaObjectNode = Node<{ object: SchemaObject }, "schemaObject">;
// An invisible, inert node marking the gap the layout reserved for a
// relationship that skips columns; being a node keeps it inside fitView.
type SchemaLaneNode = Node<Record<string, never>, "schemaLane">;
type SchemaGraphNode = SchemaObjectNode | SchemaLaneNode;
type SchemaRelationshipEdge = Edge<
	{
		relationship: SchemaRelationship;
		laneId?: string;
		/** Laid-out endpoint positions; the lane only applies while both hold. */
		sourceAt?: XYPosition;
		targetAt?: XYPosition;
	},
	"schemaRelationship"
>;

const LANE_WIDTH = 120;
const LANE_HEIGHT = 24;

type HoverTarget = { kind: "node" | "edge"; id: string };

interface Highlight {
	nodes: Set<string>;
	edges: Set<string>;
}

interface SchemaGraphContextValue {
	markerPrefix: string;
	highlight: Highlight | null;
	connectable: boolean;
	setHovered: (target: HoverTarget | null) => void;
	selectRelationship?: (index: number) => void;
}

const SchemaGraphContext = createContext<SchemaGraphContextValue>({
	markerPrefix: "",
	highlight: null,
	connectable: false,
	setHovered: () => {},
});

const EDGE_TONES = {
	plain: "color-mix(in oklab, var(--muted-foreground) 60%, transparent)",
	hierarchy: "color-mix(in oklab, var(--primary) 70%, transparent)",
	active: "var(--primary)",
} as const;
type EdgeTone = keyof typeof EDGE_TONES;

const HIDDEN_HANDLE: CSSProperties = { opacity: 0, pointerEvents: "none" };
const SOURCE_HANDLE: CSSProperties = {
	width: 12,
	height: 12,
	right: -6,
	background: "var(--primary)",
	border: "2px solid var(--background)",
};
// Covers the whole card while a link is being dragged, so dropping anywhere on
// an object connects to it.
const DROP_HANDLE: CSSProperties = {
	position: "absolute",
	inset: 0,
	width: "100%",
	height: "100%",
	minWidth: 0,
	minHeight: 0,
	transform: "none",
	borderRadius: 12,
	border: 0,
	background: "transparent",
};
const CANVAS_STYLE = {
	"--xy-background-color": "transparent",
} as CSSProperties;
const PANEL_STYLE: CSSProperties = { margin: 8 };

function isAt(node: InternalNode, position: XYPosition | undefined): boolean {
	if (!position) return false;
	const { x, y } = node.internals.positionAbsolute;
	return Math.abs(x - position.x) < 1 && Math.abs(y - position.y) < 1;
}

function nodeRect(node: InternalNode): SchemaRect {
	return {
		x: node.internals.positionAbsolute.x,
		y: node.internals.positionAbsolute.y,
		width: node.measured.width ?? SCHEMA_NODE_WIDTH,
		height: node.measured.height ?? 0,
	};
}

function RowGlyph({ role }: Readonly<{ role: SchemaRow["role"] }>) {
	if (role === "id")
		return <KeyRound className="h-3 w-3 shrink-0 text-primary" />;
	if (role === "link")
		return <Link2 className="h-3 w-3 shrink-0 text-muted-foreground" />;
	return (
		<span className="flex h-3 w-3 shrink-0 items-center justify-center">
			<span className="h-1 w-1 rounded-full bg-muted-foreground/50" />
		</span>
	);
}

const SchemaObjectCard = memo(function SchemaObjectCard({
	id,
	data,
}: NodeProps<SchemaObjectNode>) {
	const { t } = useTranslation("settings");
	const { highlight, connectable } = useContext(SchemaGraphContext);
	const connectingFrom = useConnection((connection) =>
		connection.inProgress ? connection.fromNode.id : null,
	);
	const { object } = data;
	const isObject = object.kind === "object";
	const canLink = connectable && isObject;
	const dropTarget =
		canLink && connectingFrom !== null && connectingFrom !== id;
	const dimmed = highlight !== null && !highlight.nodes.has(id);
	const Icon = isObject
		? getGraphIcon(object.icon ?? "")
		: object.kind === "external"
			? ExternalLink
			: TriangleAlert;
	const accent = object.color ?? "var(--muted-foreground)";
	const subtitle = isObject
		? object.subtitle
		: object.kind === "external"
			? (object.subtitle ?? t("anotherOntology", "Another ontology"))
			: t("notInThisOntology", "Not in this ontology");

	return (
		<div
			style={{ width: SCHEMA_NODE_WIDTH }}
			className={cn(
				"group relative rounded-xl border bg-card text-card-foreground shadow-sm transition-[opacity,box-shadow] duration-150",
				!isObject && "border-dashed bg-card/60 shadow-none",
				object.kind === "missing" && "border-destructive/60",
				dimmed && "opacity-30",
				dropTarget && "ring-2 ring-primary/60",
			)}
		>
			<div className="flex h-12 items-center gap-2.5 px-3">
				<span
					className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg"
					style={{
						backgroundColor: `color-mix(in oklab, ${accent} 18%, transparent)`,
						color: accent,
					}}
				>
					<Icon className="h-3.5 w-3.5" />
				</span>
				<div className="min-w-0 flex-1">
					<p className="truncate text-sm font-medium leading-5">
						{object.label}
					</p>
					<p className="truncate font-mono text-[10px] leading-4 text-muted-foreground">
						{subtitle}
					</p>
				</div>
			</div>
			{object.rows.length > 0 && (
				<div className="border-t py-1">
					{object.rows.map((row) => (
						<div
							key={`${row.role}:${row.name}`}
							className="flex h-[22px] items-center gap-2 px-3"
						>
							<RowGlyph role={row.role} />
							<span
								className={cn(
									"min-w-0 flex-1 truncate font-mono text-[11px]",
									row.role === "id" && "font-semibold",
								)}
							>
								{row.name}
							</span>
							{row.dataType && (
								<span className="max-w-20 shrink-0 truncate font-mono text-[10px] text-muted-foreground">
									{row.dataType}
								</span>
							)}
						</div>
					))}
					{object.hiddenRows > 0 && (
						<div className="flex h-[22px] items-center px-3 pl-8 text-[10px] text-muted-foreground">
							{t("countMoreFields", "+{{count}} more", {
								count: object.hiddenRows,
							})}
						</div>
					)}
				</div>
			)}
			<Handle
				type="target"
				position={Position.Left}
				isConnectable={false}
				style={HIDDEN_HANDLE}
			/>
			<Handle
				type="source"
				position={Position.Right}
				isConnectable={canLink}
				style={canLink ? SOURCE_HANDLE : HIDDEN_HANDLE}
				className={
					canLink
						? "opacity-0 transition-opacity group-hover:opacity-100"
						: undefined
				}
				title={canLink ? t("dragToLinkObjects", "Drag to link") : undefined}
			/>
			{dropTarget && (
				<Handle
					id="drop"
					type="target"
					position={Position.Left}
					isConnectableStart={false}
					style={DROP_HANDLE}
				/>
			)}
		</div>
	);
});

const SchemaRelationshipLine = memo(function SchemaRelationshipLine({
	id,
	source,
	target,
	data,
}: EdgeProps<SchemaRelationshipEdge>) {
	const { markerPrefix, highlight, setHovered, selectRelationship } =
		useContext(SchemaGraphContext);
	const sourceNode = useInternalNode(source);
	const targetNode = useInternalNode(target);
	const laneNode = useInternalNode(data?.laneId ?? "");
	const nodes = useNodes<SchemaGraphNode>();
	if (!data || !sourceNode || !targetNode) return null;

	const { relationship } = data;
	const obstacles = nodes.flatMap((node) =>
		node.type === "schemaObject" &&
		node.id !== source &&
		node.id !== target &&
		node.measured?.width &&
		node.measured.height
			? [
					{
						x: node.position.x,
						y: node.position.y,
						width: node.measured.width,
						height: node.measured.height,
					},
				]
			: [],
	);
	const lane =
		laneNode &&
		isAt(sourceNode, data.sourceAt) &&
		isAt(targetNode, data.targetAt)
			? {
					x: laneNode.internals.positionAbsolute.x + LANE_WIDTH / 2,
					y: laneNode.internals.positionAbsolute.y + LANE_HEIGHT / 2,
				}
			: undefined;
	const geometry =
		source === target
			? loopGeometry(nodeRect(sourceNode), relationship.loop)
			: routedCurveGeometry(
					nodeRect(sourceNode),
					nodeRect(targetNode),
					relationship.offset,
					obstacles,
					lane,
				);
	const active = highlight?.edges.has(id) ?? false;
	const dimmed = highlight !== null && !active;
	const tone: EdgeTone = active
		? "active"
		: relationship.containment
			? "hierarchy"
			: "plain";

	return (
		<>
			<BaseEdge
				path={geometry.path}
				markerEnd={`url(#${markerPrefix}-arrow-${tone})`}
				markerStart={
					relationship.containment
						? `url(#${markerPrefix}-diamond-${tone})`
						: undefined
				}
				interactionWidth={18}
				style={{
					stroke: EDGE_TONES[tone],
					strokeWidth: active ? 2 : 1.5,
					strokeDasharray: relationship.external ? "6 4" : undefined,
					opacity: dimmed ? 0.2 : 1,
					transition: "opacity 150ms",
				}}
			/>
			<EdgeLabelRenderer>
				<button
					type="button"
					className={cn(
						"nodrag nopan absolute flex items-center gap-1 rounded-md border bg-background px-1.5 py-0.5 font-mono text-[10px] font-medium shadow-sm transition-opacity",
						relationship.containment && "border-primary/40",
						active && "border-primary text-primary",
						dimmed && "opacity-30",
					)}
					style={{
						transform: `translate(-50%, -50%) translate(${geometry.labelX}px, ${geometry.labelY}px)`,
						pointerEvents: "all",
					}}
					title={relationship.join}
					onMouseEnter={() => setHovered({ kind: "edge", id })}
					onMouseLeave={() => setHovered(null)}
					onClick={() => selectRelationship?.(relationship.index)}
				>
					{relationship.containment && <GitBranch className="h-3 w-3" />}
					{relationship.label}
				</button>
			</EdgeLabelRenderer>
		</>
	);
});

function SchemaLane() {
	return <div style={{ width: LANE_WIDTH, height: LANE_HEIGHT }} />;
}

const NODE_TYPES = { schemaObject: SchemaObjectCard, schemaLane: SchemaLane };
const EDGE_TYPES = { schemaRelationship: SchemaRelationshipLine };

function SchemaMarkers({ prefix }: Readonly<{ prefix: string }>) {
	return (
		<svg className="pointer-events-none absolute h-0 w-0" aria-hidden="true">
			<defs>
				{(Object.keys(EDGE_TONES) as EdgeTone[]).map((tone) => (
					<Fragment key={tone}>
						<marker
							id={`${prefix}-arrow-${tone}`}
							viewBox="0 0 10 10"
							refX="10"
							refY="5"
							markerWidth="9"
							markerHeight="9"
							markerUnits="userSpaceOnUse"
							orient="auto"
						>
							<path d="M0,0 L10,5 L0,10 z" style={{ fill: EDGE_TONES[tone] }} />
						</marker>
						<marker
							id={`${prefix}-diamond-${tone}`}
							viewBox="0 0 14 8"
							refX="0"
							refY="4"
							markerWidth="14"
							markerHeight="8"
							markerUnits="userSpaceOnUse"
							orient="auto"
						>
							<path
								d="M0,4 L7,0 L14,4 L7,8 z"
								style={{ fill: EDGE_TONES[tone] }}
							/>
						</marker>
					</Fragment>
				))}
			</defs>
		</svg>
	);
}

function SchemaLegend({
	hasHierarchy,
	hasExternal,
}: Readonly<{ hasHierarchy: boolean; hasExternal: boolean }>) {
	const { t } = useTranslation("settings");
	return (
		<div className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border bg-background/85 px-2 py-1 text-[10px] text-muted-foreground backdrop-blur">
			<span className="flex items-center gap-1">
				<KeyRound className="h-3 w-3 text-primary" />
				{t("identity", "Identity")}
			</span>
			<span className="flex items-center gap-1">
				<Link2 className="h-3 w-3" />
				{t("joinColumn", "Join column")}
			</span>
			{hasHierarchy && (
				<span className="flex items-center gap-1">
					<GitBranch className="h-3 w-3 text-primary" />
					{t("hierarchy", "Hierarchy")}
				</span>
			)}
			{hasExternal && (
				<span className="flex items-center gap-1">
					<ExternalLink className="h-3 w-3" />
					{t("otherOntology", "Other ontology")}
				</span>
			)}
		</div>
	);
}

function SchemaGraphCanvas({
	nodes,
	edges,
	externalTargets,
	expanded,
	onExpand,
	onSelectObject,
	onSelectRelationship,
	onConnect,
}: Readonly<
	OntologySchemaGraphProps & { expanded: boolean; onExpand?: () => void }
>) {
	const { t } = useTranslation("settings");
	const { resolvedTheme } = useTheme();
	const colorMode: ColorMode = resolvedTheme === "dark" ? "dark" : "light";
	const markerPrefix = `schema-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
	const { fitView, zoomIn, zoomOut } = useReactFlow();

	const model = useMemo(
		() => buildSchemaModel(nodes, edges, externalTargets),
		[edges, externalTargets, nodes],
	);

	const { layoutNodes, flowEdges } = useMemo(() => {
		const { positions, lanes } = layoutSchema(
			model.objects.map((object) => ({
				id: object.id,
				width: SCHEMA_NODE_WIDTH,
				height: object.height,
			})),
			model.relationships,
			{ laneHeight: LANE_HEIGHT },
		);
		const objectNodes: SchemaGraphNode[] = model.objects.map((object) => ({
			id: object.id,
			type: "schemaObject",
			position: positions.get(object.id) ?? { x: 0, y: 0 },
			data: { object },
		}));
		const laneNodes: SchemaGraphNode[] = [...lanes].map(([edgeId, center]) => ({
			id: `lane:${edgeId}`,
			type: "schemaLane",
			position: {
				x: center.x - LANE_WIDTH / 2,
				y: center.y - LANE_HEIGHT / 2,
			},
			data: {},
			draggable: false,
			connectable: false,
			focusable: false,
			style: { pointerEvents: "none" },
		}));
		const relationshipEdges: SchemaRelationshipEdge[] = model.relationships.map(
			(relationship) => ({
				id: relationship.id,
				type: "schemaRelationship",
				source: relationship.source,
				target: relationship.target,
				data: {
					relationship,
					laneId: lanes.has(relationship.id)
						? `lane:${relationship.id}`
						: undefined,
					sourceAt: positions.get(relationship.source),
					targetAt: positions.get(relationship.target),
				},
			}),
		);
		return {
			layoutNodes: [...objectNodes, ...laneNodes],
			flowEdges: relationshipEdges,
		};
	}, [model]);

	// Objects the user dragged keep their spot across edits until "Tidy up".
	const pinnedRef = useRef(new Map<string, XYPosition>());
	const [flowNodes, setFlowNodes, onNodesChange] =
		useNodesState<SchemaGraphNode>(layoutNodes);
	useEffect(() => {
		setFlowNodes(
			layoutNodes.map((node) => {
				const pinned = pinnedRef.current.get(node.id);
				return pinned ? { ...node, position: pinned } : node;
			}),
		);
	}, [layoutNodes, setFlowNodes]);

	const fitOptions = useMemo(
		() => ({ padding: 0.18, maxZoom: expanded ? 1.25 : 1 }),
		[expanded],
	);
	const structureKey = useMemo(
		() =>
			[
				...model.objects.map((object) => object.id),
				...model.relationships.map(
					(relationship) => `${relationship.source}>${relationship.target}`,
				),
			].join("|"),
		[model],
	);
	// xyflow fits against the nodes already in its store, so a refit has to wait
	// for the commit that hands it the re-laid-out nodes.
	const refitPendingRef = useRef(false);
	const initialStructureRef = useRef(structureKey);
	useEffect(() => {
		if (structureKey !== initialStructureRef.current) {
			refitPendingRef.current = true;
		}
	}, [structureKey]);
	useEffect(() => {
		void flowNodes;
		if (!refitPendingRef.current) return;
		refitPendingRef.current = false;
		void fitView({ ...fitOptions, duration: 200 });
	}, [fitOptions, fitView, flowNodes]);

	const tidyUp = useCallback(() => {
		pinnedRef.current.clear();
		setFlowNodes(layoutNodes);
		void fitView({ ...fitOptions, duration: 200 });
	}, [fitOptions, fitView, layoutNodes, setFlowNodes]);

	const [hovered, setHovered] = useState<HoverTarget | null>(null);
	const highlight = useMemo<Highlight | null>(() => {
		if (!hovered) return null;
		const lit: Highlight = { nodes: new Set(), edges: new Set() };
		if (hovered.kind === "node") lit.nodes.add(hovered.id);
		for (const edge of flowEdges) {
			const touches =
				hovered.kind === "node"
					? edge.source === hovered.id || edge.target === hovered.id
					: edge.id === hovered.id;
			if (!touches) continue;
			lit.edges.add(edge.id);
			lit.nodes.add(edge.source);
			lit.nodes.add(edge.target);
		}
		return lit;
	}, [flowEdges, hovered]);

	const context = useMemo<SchemaGraphContextValue>(
		() => ({
			markerPrefix,
			highlight,
			connectable: Boolean(onConnect),
			setHovered,
			selectRelationship: onSelectRelationship,
		}),
		[highlight, markerPrefix, onConnect, onSelectRelationship],
	);

	const handleConnect = useCallback(
		(connection: Connection) => {
			const byId = new Map(model.objects.map((object) => [object.id, object]));
			const source = byId.get(connection.source)?.mapping;
			const target = byId.get(connection.target)?.mapping;
			if (source && target) onConnect?.(source, target);
		},
		[model, onConnect],
	);

	const hasHierarchy = model.relationships.some(
		(relationship) => relationship.containment,
	);
	const hasExternal = model.relationships.some(
		(relationship) => relationship.external,
	);

	return (
		<SchemaGraphContext.Provider value={context}>
			<SchemaMarkers prefix={markerPrefix} />
			<ReactFlow<SchemaGraphNode, SchemaRelationshipEdge>
				nodes={flowNodes}
				edges={flowEdges}
				nodeTypes={NODE_TYPES}
				edgeTypes={EDGE_TYPES}
				onNodesChange={onNodesChange}
				onNodeDragStop={(_, node) =>
					pinnedRef.current.set(node.id, node.position)
				}
				onNodeClick={(_, node) => {
					if (node.type !== "schemaObject") return;
					const mapping = node.data.object.mapping;
					if (mapping) onSelectObject?.(mapping);
				}}
				onEdgeClick={(_, edge) => {
					if (edge.data) onSelectRelationship?.(edge.data.relationship.index);
				}}
				onNodeMouseEnter={(_, node) => {
					if (node.type === "schemaObject") {
						setHovered({ kind: "node", id: node.id });
					}
				}}
				onNodeMouseLeave={() => setHovered(null)}
				onEdgeMouseEnter={(_, edge) =>
					setHovered({ kind: "edge", id: edge.id })
				}
				onEdgeMouseLeave={() => setHovered(null)}
				onConnect={handleConnect}
				nodesConnectable={Boolean(onConnect)}
				elementsSelectable={false}
				edgesFocusable={false}
				colorMode={colorMode}
				fitView
				fitViewOptions={fitOptions}
				minZoom={0.2}
				maxZoom={1.75}
				zoomOnScroll={expanded}
				preventScrolling={expanded}
				zoomOnDoubleClick={false}
				connectionRadius={36}
				connectionLineStyle={{ stroke: "var(--primary)", strokeWidth: 1.5 }}
				proOptions={{ hideAttribution: true }}
				style={CANVAS_STYLE}
			>
				<Background variant={BackgroundVariant.Dots} gap={16} size={1} />
				<Panel position="top-right" className="flex gap-1" style={PANEL_STYLE}>
					<Button
						variant="outline"
						size="icon"
						className="h-7 w-7 bg-background/85 backdrop-blur"
						onClick={() => void zoomOut({ duration: 150 })}
						title={t("zoomOut", "Zoom out")}
					>
						<Minus className="h-3.5 w-3.5" />
					</Button>
					<Button
						variant="outline"
						size="icon"
						className="h-7 w-7 bg-background/85 backdrop-blur"
						onClick={() => void zoomIn({ duration: 150 })}
						title={t("zoomIn", "Zoom in")}
					>
						<Plus className="h-3.5 w-3.5" />
					</Button>
					<Button
						variant="outline"
						size="icon"
						className="h-7 w-7 bg-background/85 backdrop-blur"
						onClick={() => void fitView({ ...fitOptions, duration: 200 })}
						title={t("fitToView", "Fit to view")}
					>
						<Scan className="h-3.5 w-3.5" />
					</Button>
					<Button
						variant="outline"
						size="icon"
						className="h-7 w-7 bg-background/85 backdrop-blur"
						onClick={tidyUp}
						title={t("tidyUpLayout", "Tidy up layout")}
					>
						<LayoutGrid className="h-3.5 w-3.5" />
					</Button>
					{onExpand && (
						<Button
							variant="outline"
							size="icon"
							className="h-7 w-7 bg-background/85 backdrop-blur"
							onClick={onExpand}
							title={t("expandDiagram", "Expand diagram")}
						>
							<Maximize2 className="h-3.5 w-3.5" />
						</Button>
					)}
				</Panel>
				<Panel position="bottom-left" style={PANEL_STYLE}>
					<SchemaLegend hasHierarchy={hasHierarchy} hasExternal={hasExternal} />
				</Panel>
			</ReactFlow>
		</SchemaGraphContext.Provider>
	);
}

/**
 * Scrolls the editor a diagram click points at into view and flags it for a
 * brief highlight. `domId(key)` goes on the element, `revealedKey` is the key
 * currently flashing.
 */
export function useRevealTarget() {
	const prefix = `reveal-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
	const [revealed, setRevealed] = useState<{
		key: string;
		nonce: number;
	} | null>(null);
	useEffect(() => {
		if (!revealed) return;
		document
			.getElementById(`${prefix}-${revealed.key}`)
			?.scrollIntoView({ behavior: "smooth", block: "center" });
		const timer = setTimeout(() => setRevealed(null), 1600);
		return () => clearTimeout(timer);
	}, [prefix, revealed]);
	const reveal = useCallback(
		(key: string) => setRevealed({ key, nonce: Date.now() }),
		[],
	);
	const domId = useCallback((key: string) => `${prefix}-${key}`, [prefix]);
	return { reveal, domId, revealedKey: revealed?.key };
}

/**
 * The ontology as a schema diagram: one card per object type with its identity
 * and join columns, one labelled arrow per relationship. Hierarchy edges carry
 * a diamond at the parent, children in other ontologies show as dashed ghosts.
 */
export function OntologySchemaGraph(props: Readonly<OntologySchemaGraphProps>) {
	const { t } = useTranslation("settings");
	const [expanded, setExpanded] = useState(false);
	const {
		className,
		title,
		onSelectObject,
		onSelectRelationship,
		onConnect,
		...graph
	} = props;
	// Every action jumps to an editor below the diagram, so the expanded view
	// gets out of the way first.
	const expandedHandlers = useMemo(
		() => ({
			onSelectObject: onSelectObject
				? (object: NodeLabelMapping) => {
						setExpanded(false);
						onSelectObject(object);
					}
				: undefined,
			onSelectRelationship: onSelectRelationship
				? (index: number) => {
						setExpanded(false);
						onSelectRelationship(index);
					}
				: undefined,
			onConnect: onConnect
				? (source: NodeLabelMapping, target: NodeLabelMapping) => {
						setExpanded(false);
						onConnect(source, target);
					}
				: undefined,
		}),
		[onConnect, onSelectObject, onSelectRelationship],
	);

	return (
		<>
			<div
				className={cn(
					"relative h-95 overflow-hidden rounded-xl border bg-muted/20",
					className,
				)}
			>
				<ReactFlowProvider>
					<SchemaGraphCanvas
						{...graph}
						onSelectObject={onSelectObject}
						onSelectRelationship={onSelectRelationship}
						onConnect={onConnect}
						expanded={false}
						onExpand={() => setExpanded(true)}
					/>
				</ReactFlowProvider>
			</div>
			<Dialog open={expanded} onOpenChange={setExpanded}>
				<DialogContent className="flex h-[88vh] flex-col gap-3 sm:max-w-[min(96vw,1600px)]">
					<DialogHeader>
						<DialogTitle>
							{title ?? t("schemaDiagram", "Schema diagram")}
						</DialogTitle>
						<DialogDescription>
							{t(
								"objectTypesAndHowTheyRelateScrollToZoomDragToRearrange",
								"Object types and how they relate. Scroll to zoom, drag to rearrange.",
							)}
						</DialogDescription>
					</DialogHeader>
					<div className="relative min-h-0 flex-1 overflow-hidden rounded-xl border bg-muted/20">
						{expanded && (
							<ReactFlowProvider>
								<SchemaGraphCanvas {...graph} {...expandedHandlers} expanded />
							</ReactFlowProvider>
						)}
					</div>
				</DialogContent>
			</Dialog>
		</>
	);
}
