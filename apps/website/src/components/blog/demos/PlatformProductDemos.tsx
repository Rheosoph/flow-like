"use client";

import {
	Background,
	BackgroundVariant,
	BaseEdge,
	Controls,
	type Edge,
	EdgeLabelRenderer,
	type EdgeProps,
	Handle,
	MarkerType,
	type Node,
	type NodeProps,
	Position,
	ReactFlow,
	type ReactFlowInstance,
	getBezierPath,
	useNodesState,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import {
	Archive,
	ArrowDownLeft,
	ArrowRight,
	ArrowUpRight,
	Blocks,
	BookOpen,
	Box,
	Boxes,
	Check,
	CheckCircle2,
	ChevronDown,
	ChevronRight,
	CircleDot,
	Clock,
	Cloud,
	Code2,
	Database,
	Eye,
	EyeOff,
	FileKey,
	FileText,
	GitBranch,
	Layers,
	Layers3,
	LayoutDashboard,
	List,
	Maximize2,
	MoreVertical,
	Network,
	PanelLeftClose,
	Pencil,
	Play,
	Plus,
	RefreshCw,
	Route,
	Save,
	Search,
	Share2,
	Shield,
	ShieldCheck,
	SquareTerminal,
	Table2,
	Trash2,
	Users,
	Waypoints,
	Workflow,
	X,
	XCircle,
	Zap,
} from "lucide-react";
import {
	type ReactNode,
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { ProductDemoFrame, cn } from "./ProductDemoFrame";

function ProductButton({
	children,
	onClick,
	variant = "default",
	disabled,
	className,
	ariaLabel,
}: Readonly<{
	children: ReactNode;
	onClick?: () => void;
	variant?: "default" | "outline" | "ghost" | "destructive" | "secondary";
	disabled?: boolean;
	className?: string;
	ariaLabel?: string;
}>) {
	return (
		<button
			type="button"
			onClick={onClick}
			disabled={disabled}
			aria-label={ariaLabel}
			className={cn(
				"inline-flex h-9 items-center justify-center gap-2 rounded-md px-3 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50",
				variant === "default" &&
					"bg-primary text-primary-foreground hover:bg-primary/90",
				variant === "outline" && "border bg-background hover:bg-muted/60",
				variant === "ghost" && "hover:bg-muted/70",
				variant === "secondary" && "bg-secondary text-secondary-foreground",
				variant === "destructive" &&
					"bg-destructive text-destructive-foreground hover:bg-destructive/90",
				className,
			)}
		>
			{children}
		</button>
	);
}

function ProductBadge({
	children,
	variant = "outline",
	className,
}: Readonly<{
	children: ReactNode;
	variant?: "outline" | "secondary" | "destructive" | "default";
	className?: string;
}>) {
	return (
		<span
			className={cn(
				"inline-flex items-center rounded-md px-2 py-0.5 text-[11px] font-medium",
				variant === "outline" && "border bg-background",
				variant === "secondary" && "bg-secondary text-secondary-foreground",
				variant === "destructive" &&
					"bg-destructive text-destructive-foreground",
				variant === "default" && "bg-primary text-primary-foreground",
				className,
			)}
		>
			{children}
		</span>
	);
}

function ProductCard({
	children,
	className,
}: Readonly<{ children: ReactNode; className?: string }>) {
	return (
		<div
			className={cn(
				"rounded-xl border bg-card text-card-foreground",
				className,
			)}
		>
			{children}
		</div>
	);
}

function ProductAvatar({
	label,
	className,
	tone = "primary",
}: Readonly<{
	label: string;
	className?: string;
	tone?: "primary" | "violet" | "cyan" | "amber" | "emerald";
}>) {
	const tones = {
		primary: "bg-primary/10 text-primary",
		violet: "bg-violet-500/15 text-violet-600 dark:text-violet-300",
		cyan: "bg-cyan-500/15 text-cyan-600 dark:text-cyan-300",
		amber: "bg-amber-500/15 text-amber-600 dark:text-amber-300",
		emerald: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-300",
	};
	return (
		<span
			className={cn(
				"flex size-10 shrink-0 items-center justify-center rounded-lg text-xs font-bold",
				tones[tone],
				className,
			)}
		>
			{label}
		</span>
	);
}

function CapabilityChip({
	icon: Icon,
	label,
	access,
}: Readonly<{
	icon: typeof Database;
	label: string;
	access?: "Read" | "Read/Write";
}>) {
	const AccessIcon = access === "Read/Write" ? Pencil : Eye;
	return (
		<span className="flex items-center gap-1 rounded bg-muted px-1.5 py-0.5 text-[10px] leading-none text-muted-foreground">
			<Icon className="size-2.5" />
			{label}
			{access ? <AccessIcon className="size-2.5" /> : null}
			{access}
		</span>
	);
}

type ConnectionFixture = {
	id: string;
	name: string;
	role: string;
	description: string;
	tone: "primary" | "violet" | "cyan" | "amber" | "emerald";
	capabilities: string[];
};

const connectedRows: ConnectionFixture[] = [
	{
		id: "risk",
		name: "Risk Engine",
		role: "Workflow executor",
		description: "Policy checks and fraud screening",
		tone: "violet" as const,
		capabilities: ["Events", "DB"],
	},
	{
		id: "reporting",
		name: "Reporting",
		role: "Operations reader",
		description: "Operational dashboards and exports",
		tone: "cyan" as const,
		capabilities: ["DB", "Files"],
	},
];

function ConnectionRow({ row }: Readonly<{ row: ConnectionFixture }>) {
	return (
		<div className="flex items-center justify-between rounded-lg border p-4 transition-colors hover:bg-muted/50">
			<div className="flex min-w-0 flex-1 items-center gap-3">
				<ProductAvatar
					label={row.name.slice(0, 2).toUpperCase()}
					tone={row.tone}
				/>
				<div className="min-w-0 flex-1">
					<h4 className="truncate text-sm font-medium">{row.name}</h4>
					<div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
						<span className="flex items-center gap-1">
							<Shield className="size-3" />
							{row.role}
						</span>
						<span>Connected Jul 18</span>
					</div>
					<div className="mt-1.5 flex flex-wrap items-center gap-1">
						{row.capabilities.includes("Events") ? (
							<CapabilityChip icon={Zap} label="Events" />
						) : null}
						{row.capabilities.includes("DB") ? (
							<CapabilityChip icon={Database} label="DB" access="Read" />
						) : null}
						{row.capabilities.includes("Files") ? (
							<CapabilityChip icon={FileText} label="Files" access="Read" />
						) : null}
					</div>
					<p className="mt-1 truncate text-xs text-muted-foreground">
						{row.description}
					</p>
				</div>
			</div>
			<ProductButton
				variant="ghost"
				className="size-9 p-0"
				ariaLabel="Connection actions"
			>
				<MoreVertical className="size-4" />
			</ProductButton>
		</div>
	);
}

export function AppConnectionsDemo() {
	const [view, setView] = useState<"list" | "graph">("list");
	const [pending, setPending] = useState(true);
	const [approveOpen, setApproveOpen] = useState(false);
	const [role, setRole] = useState("Operations reader");

	return (
		<ProductDemoFrame source="packages/ui/components/settings/team/app-connection-management.tsx">
			<div className="relative space-y-6 rounded-xl border bg-background p-4 sm:p-6">
				<div className="inline-flex items-center gap-1 rounded-lg border bg-muted/40 p-1">
					<ProductButton
						variant={view === "list" ? "secondary" : "ghost"}
						className="h-8 px-2.5"
						onClick={() => setView("list")}
					>
						<List className="size-4" /> List
					</ProductButton>
					<ProductButton
						variant={view === "graph" ? "secondary" : "ghost"}
						className="h-8 px-2.5"
						onClick={() => setView("graph")}
					>
						<Waypoints className="size-4" /> Process graph
					</ProductButton>
				</div>

				{view === "graph" ? (
					<div className="rounded-lg border border-dashed p-8 text-center">
						<Waypoints className="mx-auto size-7 text-muted-foreground" />
						<p className="mt-3 text-sm font-medium">
							Process graph is the next live surface
						</p>
						<p className="mt-1 text-xs text-muted-foreground">
							Switch to the dedicated lineage demo below for the full graph and
							case view.
						</p>
					</div>
				) : (
					<>
						<ProductCard>
							<div className="flex flex-col gap-3 border-b p-5 sm:flex-row sm:items-center sm:justify-between">
								<div>
									<h3 className="flex items-center gap-2 font-semibold">
										<ArrowDownLeft className="size-5" /> Apps With Access
									</h3>
									<p className="mt-1 text-sm text-muted-foreground">
										Apps that can work with this app's databases, data and
										events.
									</p>
								</div>
								<ProductButton>
									<Plus className="size-4" /> Grant App Access
								</ProductButton>
							</div>
							<div className="space-y-5 p-5">
								{pending ? (
									<div className="space-y-3">
										<h4 className="flex items-center gap-2 text-sm font-medium">
											<Clock className="size-4" /> Pending Requests
											<ProductBadge variant="secondary">1</ProductBadge>
										</h4>
										<div className="group rounded-lg border border-l-4 border-l-amber-400 bg-card p-4 transition-shadow hover:shadow-md">
											<div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
												<div className="flex min-w-0 flex-1 items-center gap-3">
													<ProductAvatar label="II" />
													<div className="min-w-0">
														<div className="flex items-center gap-2">
															<h4 className="truncate text-sm font-medium">
																Invoice Intake
															</h4>
															<span className="rounded-full bg-amber-100 px-2 py-0.5 text-xs text-amber-800 dark:bg-amber-900/30 dark:text-amber-300">
																Pending
															</span>
														</div>
														<p className="mt-1 flex items-center gap-1 text-xs text-muted-foreground">
															<Clock className="size-3" /> Requested today
														</p>
													</div>
												</div>
												<div className="flex gap-2">
													<ProductButton
														className="h-8"
														onClick={() => setApproveOpen(true)}
													>
														<Check className="size-4" /> Approve
													</ProductButton>
													<ProductButton
														variant="destructive"
														className="h-8"
														onClick={() => setPending(false)}
													>
														<X className="size-4" /> Reject
													</ProductButton>
												</div>
											</div>
											<div className="mt-3 rounded-lg border bg-muted/30 p-3 text-sm text-muted-foreground">
												“Process new invoices and hand approved orders to
												fulfilment.”
											</div>
										</div>
									</div>
								) : null}
								<div className="space-y-3">
									<h4 className="flex items-center gap-2 text-sm font-medium">
										<Blocks className="size-4" /> Connected Apps
									</h4>
									{connectedRows.map((row) => (
										<ConnectionRow key={row.id} row={row} />
									))}
								</div>
							</div>
						</ProductCard>

						<ProductCard>
							<div className="flex flex-col gap-3 border-b p-5 sm:flex-row sm:items-center sm:justify-between">
								<div>
									<h3 className="flex items-center gap-2 font-semibold">
										<ArrowUpRight className="size-5" /> Outgoing Access
									</h3>
									<p className="mt-1 text-sm text-muted-foreground">
										Apps this app can access and requests sent in its name.
									</p>
								</div>
								<ProductButton variant="outline">
									<ArrowRight className="size-4" /> Request Access
								</ProductButton>
							</div>
							<div className="p-5">
								<ConnectionRow
									row={{
										id: "fulfilment",
										name: "Fulfilment Core",
										role: "Event caller",
										description: "Order creation and dispatch",
										tone: "emerald",
										capabilities: ["Events"],
									}}
								/>
							</div>
						</ProductCard>
					</>
				)}

				{approveOpen ? (
					<div className="absolute inset-0 z-20 flex items-center justify-center rounded-xl bg-background/75 p-4 backdrop-blur-sm">
						<dialog
							open
							className="w-full max-w-md rounded-xl border bg-background p-5 shadow-xl"
						>
							<div className="mx-auto flex size-12 items-center justify-center rounded-full bg-primary/10">
								<Shield className="size-6 text-primary" />
							</div>
							<h3 className="mt-3 text-center text-xl font-semibold">
								Approve Request
							</h3>
							<p className="mt-1 text-center text-sm text-muted-foreground">
								Select a target-app role for Invoice Intake.
							</p>
							<label className="mt-5 block text-sm font-medium">
								Role
								<select
									value={role}
									onChange={(event) => setRole(event.target.value)}
									className="mt-2 h-10 w-full rounded-md border bg-background px-3"
								>
									<option>Operations reader</option>
									<option>Workflow executor</option>
									<option>Data contributor</option>
								</select>
							</label>
							<div className="mt-5 flex justify-end gap-2">
								<ProductButton
									variant="outline"
									onClick={() => setApproveOpen(false)}
								>
									Cancel
								</ProductButton>
								<ProductButton
									onClick={() => {
										setPending(false);
										setApproveOpen(false);
									}}
								>
									Approve
								</ProductButton>
							</div>
						</dialog>
					</div>
				) : null}
			</div>
		</ProductDemoFrame>
	);
}

type LineageNode = {
	id: string;
	name: string;
	initials: string;
	note: string;
	tone: "primary" | "violet" | "cyan" | "amber" | "emerald";
};

const lineageNodes: LineageNode[] = [
	{
		id: "intake",
		name: "Invoice Intake",
		initials: "II",
		note: "Normalizes incoming invoices",
		tone: "cyan",
	},
	{
		id: "risk",
		name: "Risk Engine",
		initials: "RE",
		note: "Runs policy and fraud screening",
		tone: "violet",
	},
	{
		id: "erp",
		name: "ERP Sync",
		initials: "ES",
		note: "Creates approved orders",
		tone: "amber",
	},
	{
		id: "ship",
		name: "Fulfilment",
		initials: "FC",
		note: "Owns dispatch and status",
		tone: "emerald",
	},
];

type ProcessGraphNodeData = {
	app: LineageNode;
};
type ProcessGraphFlowNode = Node<ProcessGraphNodeData, "processApp">;
type ProcessGraphEdgeData = {
	observed: boolean;
	runCount?: number;
	label?: string;
};
type ProcessGraphFlowEdge = Edge<ProcessGraphEdgeData, "connection">;

const PROCESS_NODE_POSITIONS: Record<string, { x: number; y: number }> = {
	intake: { x: 0, y: 140 },
	risk: { x: 330, y: 140 },
	erp: { x: 660, y: 20 },
	ship: { x: 660, y: 260 },
};

const INITIAL_PROCESS_FLOW_NODES: ProcessGraphFlowNode[] = lineageNodes.map(
	(app) => ({
		id: app.id,
		type: "processApp",
		position: PROCESS_NODE_POSITIONS[app.id],
		data: { app },
		deletable: false,
	}),
);

const PROCESS_FLOW_EDGES: ProcessGraphFlowEdge[] = [
	{
		id: "intake-risk",
		source: "intake",
		target: "risk",
		type: "connection",
		animated: true,
		selectable: false,
		focusable: false,
		deletable: false,
		data: { observed: true, runCount: 244, label: "Invoice reviewed" },
		style: { stroke: "var(--primary)", strokeWidth: 2 },
		markerEnd: { type: MarkerType.ArrowClosed, color: "var(--primary)" },
	},
	{
		id: "risk-erp",
		source: "risk",
		target: "erp",
		type: "connection",
		animated: true,
		selectable: false,
		focusable: false,
		deletable: false,
		data: { observed: true, runCount: 186, label: "Approved" },
		style: { stroke: "var(--primary)", strokeWidth: 2 },
		markerEnd: { type: MarkerType.ArrowClosed, color: "var(--primary)" },
	},
	{
		id: "risk-ship",
		source: "risk",
		target: "ship",
		type: "connection",
		selectable: false,
		focusable: false,
		deletable: false,
		data: { observed: false, label: "Operations reader" },
		style: { stroke: "var(--muted-foreground)", strokeWidth: 1.5 },
		markerEnd: {
			type: MarkerType.ArrowClosed,
			color: "var(--muted-foreground)",
		},
	},
];

const ProcessAppNode = memo(
	({ data, selected }: NodeProps<ProcessGraphFlowNode>) => {
		const app = data.app;
		return (
			<div
				className={cn(
					"w-52 cursor-grab rounded-lg border bg-card p-3 shadow-sm transition-shadow hover:shadow-md active:cursor-grabbing",
					app.id === "risk" && "ring-2 ring-primary",
					selected && app.id !== "risk" && "ring-1 ring-ring",
				)}
			>
				<Handle
					type="target"
					position={Position.Left}
					isConnectable={false}
					className="!size-2 !border-0 !bg-muted-foreground"
				/>
				<div className="flex min-w-0 items-center gap-2">
					<ProductAvatar
						label={app.initials}
						tone={app.tone}
						className="size-8 rounded-md"
					/>
					<div className="min-w-0 flex-1">
						<p className="truncate text-sm font-medium">{app.name}</p>
						{app.id === "risk" ? (
							<p className="text-[10px] font-medium text-primary">This app</p>
						) : null}
					</div>
					<ProductBadge
						variant="secondary"
						className="shrink-0 gap-1 px-1.5 text-[10px]"
					>
						<BookOpen className="size-3" /> 1
					</ProductBadge>
				</div>
				<Handle
					type="source"
					position={Position.Right}
					isConnectable={false}
					className="!size-2 !border-0 !bg-muted-foreground"
				/>
			</div>
		);
	},
);
ProcessAppNode.displayName = "ProcessAppNode";

const ProcessConnectionEdge = memo(
	({
		id,
		sourceX,
		sourceY,
		targetX,
		targetY,
		sourcePosition,
		targetPosition,
		markerEnd,
		style,
		data,
	}: EdgeProps<ProcessGraphFlowEdge>) => {
		const [edgePath, labelX, labelY] = getBezierPath({
			sourceX,
			sourceY,
			targetX,
			targetY,
			sourcePosition,
			targetPosition,
		});
		return (
			<>
				<BaseEdge
					id={id}
					path={edgePath}
					markerEnd={markerEnd}
					style={style}
					interactionWidth={24}
				/>
				{data?.label ? (
					<EdgeLabelRenderer>
						<div
							className="pointer-events-none absolute flex flex-col items-center rounded-md border bg-background/90 px-1.5 py-1 text-center shadow-sm backdrop-blur-sm"
							style={{
								transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
							}}
						>
							<span className="max-w-28 truncate text-[10px] font-medium leading-none">
								{data.label}
							</span>
							{data.runCount !== undefined ? (
								<span className="mt-0.5 text-[9px] leading-none text-muted-foreground">
									{data.runCount} runs
								</span>
							) : null}
						</div>
					</EdgeLabelRenderer>
				) : null}
			</>
		);
	},
);
ProcessConnectionEdge.displayName = "ProcessConnectionEdge";

const processNodeTypes = { processApp: ProcessAppNode };
const processEdgeTypes = { connection: ProcessConnectionEdge };

function ProcessLineageCanvas({
	selected,
	onSelect,
	refreshToken,
	expanded,
}: Readonly<{
	selected: string | null;
	onSelect: (id: string | null) => void;
	refreshToken: number;
	expanded: boolean;
}>) {
	const canvasRef = useRef<HTMLDivElement>(null);
	const instanceRef = useRef<ReactFlowInstance<
		ProcessGraphFlowNode,
		ProcessGraphFlowEdge
	> | null>(null);
	const [nodes, setNodes, onNodesChange] = useNodesState(
		INITIAL_PROCESS_FLOW_NODES,
	);

	useEffect(() => {
		setNodes((current) =>
			current.map((node) => ({ ...node, selected: node.id === selected })),
		);
	}, [selected, setNodes]);

	useEffect(() => {
		if (refreshToken === 0) return;
		setNodes((current) =>
			INITIAL_PROCESS_FLOW_NODES.map((node) => ({
				...node,
				position: { ...node.position },
				selected: current.some(
					(candidate) => candidate.id === node.id && candidate.selected,
				),
			})),
		);
		const timer = setTimeout(
			() => instanceRef.current?.fitView({ padding: 0.2, maxZoom: 1 }),
			40,
		);
		return () => clearTimeout(timer);
	}, [refreshToken, setNodes]);

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas || typeof ResizeObserver === "undefined") return;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const observer = new ResizeObserver(() => {
			if (timer) clearTimeout(timer);
			timer = setTimeout(
				() =>
					instanceRef.current?.fitView({
						padding: 0.2,
						duration: 180,
						maxZoom: 1,
					}),
				60,
			);
		});
		observer.observe(canvas);
		return () => {
			observer.disconnect();
			if (timer) clearTimeout(timer);
		};
	}, []);

	const handleInit = useCallback(
		(
			instance: ReactFlowInstance<ProcessGraphFlowNode, ProcessGraphFlowEdge>,
		) => {
			instanceRef.current = instance;
			instance.fitView({ padding: 0.2, maxZoom: 1 });
		},
		[],
	);

	return (
		<div
			ref={canvasRef}
			className={cn(
				"min-w-0 flex-1 bg-card",
				expanded ? "h-[42rem]" : "h-[30rem]",
			)}
		>
			<ReactFlow<ProcessGraphFlowNode, ProcessGraphFlowEdge>
				nodes={nodes}
				edges={PROCESS_FLOW_EDGES}
				nodeTypes={processNodeTypes}
				edgeTypes={processEdgeTypes}
				onNodesChange={onNodesChange}
				onInit={handleInit}
				onNodeClick={(_event, node) => onSelect(node.id)}
				onPaneClick={() => onSelect(null)}
				nodesDraggable
				nodesConnectable={false}
				deleteKeyCode={null}
				colorMode="light"
				zoomOnScroll={false}
				zoomOnDoubleClick={false}
				zoomOnPinch
				preventScrolling={false}
				fitView
				fitViewOptions={{ padding: 0.2, maxZoom: 1 }}
				minZoom={0.25}
				maxZoom={1.5}
				proOptions={{ hideAttribution: true }}
			>
				<Background variant={BackgroundVariant.Dots} gap={12} size={1} />
				<Controls showInteractive={false} />
			</ReactFlow>
		</div>
	);
}

function StatTile({
	icon: Icon,
	label,
	value,
	sub,
}: Readonly<{
	icon: typeof Blocks;
	label: string;
	value: string;
	sub?: string;
}>) {
	return (
		<div className="rounded-lg border bg-card px-3.5 py-3">
			<div className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
				<Icon className="size-3.5" /> {label}
			</div>
			<p className="mt-1.5 text-xl font-semibold leading-none tabular-nums">
				{value}
			</p>
			<p className="mt-1 min-h-3.5 text-[11px] leading-none text-muted-foreground">
				{sub ?? ""}
			</p>
		</div>
	);
}

export function ProcessLineageDemo() {
	const [days, setDays] = useState("30");
	const [selected, setSelected] = useState<string | null>("risk");
	const [refreshToken, setRefreshToken] = useState(0);
	const [expanded, setExpanded] = useState(false);
	const [bottomTab, setBottomTab] = useState<"cases" | "chains">("chains");
	const current = selected
		? lineageNodes.find((node) => node.id === selected)
		: undefined;

	useEffect(() => {
		const closeInspector = (event: KeyboardEvent) => {
			if (event.key === "Escape") setSelected(null);
		};
		window.addEventListener("keydown", closeInspector);
		return () => window.removeEventListener("keydown", closeInspector);
	}, []);

	return (
		<ProductDemoFrame source="packages/ui/components/settings/connections/process-graph.tsx">
			<div className="@container space-y-4 rounded-xl border bg-background p-4 sm:p-6">
				<div className="flex flex-wrap items-center gap-3">
					<select
						value={days}
						onChange={(event) => setDays(event.target.value)}
						className="h-9 w-36 rounded-md border bg-background px-3 text-sm"
					>
						<option value="7">Last 7 days</option>
						<option value="30">Last 30 days</option>
						<option value="90">Last 90 days</option>
						<option value="365">Last 365 days</option>
					</select>
					<ProductButton
						variant="outline"
						className="h-9"
						onClick={() => setRefreshToken((value) => value + 1)}
					>
						<RefreshCw className="size-4" /> Refresh
					</ProductButton>
					<ProductButton
						variant="outline"
						className="size-9 p-0"
						ariaLabel={expanded ? "Collapse graph" : "Expand graph"}
						onClick={() => setExpanded((value) => !value)}
					>
						<Maximize2
							className={cn(
								"size-4 transition-transform",
								expanded && "rotate-180",
							)}
						/>
					</ProductButton>
					<div className="ml-auto flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
						<span className="flex items-center gap-1.5">
							<span className="h-0.5 w-5 rounded bg-primary" /> Observed call
						</span>
						<span className="flex items-center gap-1.5">
							<span className="h-0.5 w-5 rounded bg-muted-foreground" />{" "}
							Connection
						</span>
						<span className="flex items-center gap-1">
							<Zap className="size-3" /> Event
						</span>
					</div>
				</div>

				<div className="grid grid-cols-2 gap-2 @2xl:grid-cols-3 @4xl:grid-cols-5">
					<StatTile icon={Blocks} label="Connected apps" value="4" />
					<StatTile
						icon={Zap}
						label="Observed runs"
						value={days === "7" ? "148" : "612"}
						sub="4 failed"
					/>
					<StatTile icon={CheckCircle2} label="Success rate" value="99.3%" />
					<StatTile
						icon={Workflow}
						label="Process cases"
						value="86"
						sub="2 failed"
					/>
					<StatTile icon={Clock} label="Avg case time" value="4.8s" />
				</div>

				<div
					className={cn(
						"flex w-full flex-col overflow-hidden rounded-lg border bg-card @6xl:flex-row",
						expanded ? "@6xl:h-[42rem]" : "@6xl:h-[30rem]",
					)}
				>
					<ProcessLineageCanvas
						selected={selected}
						onSelect={setSelected}
						refreshToken={refreshToken}
						expanded={expanded}
					/>
					{current ? (
						<aside
							aria-label="App details"
							className="shrink-0 border-t bg-card @6xl:w-80 @6xl:border-l @6xl:border-t-0"
						>
							<div className="flex h-full flex-col">
								<div className="relative shrink-0 border-b bg-gradient-to-br from-primary/10 via-transparent to-cyan-500/10 p-4">
									<ProductButton
										variant="ghost"
										className="absolute right-2 top-2 size-7 bg-background/60 p-0 backdrop-blur-sm"
										ariaLabel="Close details"
										onClick={() => setSelected(null)}
									>
										<X className="size-4" />
									</ProductButton>
									<div className="flex items-center gap-3 pr-8">
										<ProductAvatar
											label={current.initials}
											tone={current.tone}
										/>
										<div className="min-w-0">
											<h3 className="truncate font-semibold">{current.name}</h3>
											<ProductBadge variant="secondary">Observed</ProductBadge>
										</div>
									</div>
								</div>
								<div className="grid gap-5 p-4 @2xl:grid-cols-2 @6xl:grid-cols-1">
									<div>
										<h4 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
											Process note
										</h4>
										<p className="mt-2 text-sm leading-relaxed">
											{current.note}
										</p>
									</div>
									<div>
										<h4 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
											Content
										</h4>
										<div className="mt-2 flex flex-wrap gap-1.5">
											<CapabilityChip icon={Workflow} label="3 boards" />
											<CapabilityChip icon={Database} label="2 tables" />
											<CapabilityChip icon={Zap} label="4 events" />
										</div>
									</div>
								</div>
							</div>
						</aside>
					) : null}
				</div>

				<div className="flex w-fit items-center gap-1 rounded-lg border bg-card p-1">
					<ProductButton
						variant={bottomTab === "cases" ? "secondary" : "ghost"}
						className="h-7 px-2.5 text-xs"
						onClick={() => setBottomTab("cases")}
					>
						<Workflow className="size-3.5" /> Process Cases{" "}
						<span className="text-muted-foreground">86</span>
					</ProductButton>
					<ProductButton
						variant={bottomTab === "chains" ? "secondary" : "ghost"}
						className="h-7 px-2.5 text-xs"
						onClick={() => setBottomTab("chains")}
					>
						<GitBranch className="size-3.5" /> Observed Chains{" "}
						<span className="text-muted-foreground">3</span>
					</ProductButton>
				</div>
				<ProductCard>
					<div className="border-b p-4">
						<h3 className="flex items-center gap-2 font-semibold">
							{bottomTab === "chains" ? (
								<GitBranch className="size-4" />
							) : (
								<Workflow className="size-4" />
							)}
							{bottomTab === "chains" ? "Observed Chains" : "Process Cases"}
						</h3>
					</div>
					<div className="space-y-2 p-4">
						{bottomTab === "chains" ? (
							<div className="flex flex-wrap items-center justify-between gap-2 rounded-md border p-2.5 hover:bg-muted/40">
								<div className="flex flex-wrap items-center gap-1 text-sm font-medium">
									<span>Invoice Intake</span>
									<ChevronRight className="size-3.5 text-muted-foreground" />
									<span>Risk Engine</span>
									<ChevronRight className="size-3.5 text-muted-foreground" />
									<span>ERP Sync</span>
								</div>
								<div className="flex items-center gap-2 text-xs text-muted-foreground">
									<ProductBadge variant="secondary">186 runs</ProductBadge>
									<Clock className="size-3" /> 1.4s
								</div>
							</div>
						) : (
							<p className="text-sm text-muted-foreground">
								Open the dedicated Process Cases surface for searchable
								business-object traces.
							</p>
						)}
					</div>
				</ProductCard>
			</div>
		</ProductDemoFrame>
	);
}

type ProcessCase = {
	id: string;
	status: "Failed" | "Running" | "Completed";
	root: string;
	event: string;
	apps: string[];
	runs: number;
	failed: number;
	duration: string;
};

const processCases: ProcessCase[] = [
	{
		id: "order-1048",
		status: "Completed",
		root: "Invoice Intake",
		event: "Process invoice",
		apps: ["Invoice Intake", "Risk Engine", "ERP Sync", "Fulfilment"],
		runs: 7,
		failed: 0,
		duration: "4.8s",
	},
	{
		id: "order-1047",
		status: "Failed",
		root: "Invoice Intake",
		event: "Process invoice",
		apps: ["Invoice Intake", "Risk Engine", "ERP Sync"],
		runs: 5,
		failed: 1,
		duration: "3.1s",
	},
	{
		id: "order-1046",
		status: "Running",
		root: "Invoice Intake",
		event: "Process invoice",
		apps: ["Invoice Intake", "Risk Engine"],
		runs: 3,
		failed: 0,
		duration: "1.4s",
	},
];

function CaseStatus({ status }: Readonly<{ status: ProcessCase["status"] }>) {
	const Icon =
		status === "Failed" ? XCircle : status === "Running" ? Play : CheckCircle2;
	return (
		<span
			className={cn(
				"flex shrink-0 items-center gap-1 text-[11px] font-medium",
				status === "Failed" ? "text-destructive" : "text-muted-foreground",
			)}
		>
			<Icon className="size-3.5" /> {status}
		</span>
	);
}

export function ProcessCasesDemo() {
	const [filter, setFilter] = useState<"all" | ProcessCase["status"]>("all");
	const [search, setSearch] = useState("");
	const [expanded, setExpanded] = useState<string | null>("order-1048");
	const visible = processCases.filter(
		(entry) =>
			(filter === "all" || entry.status === filter) &&
			`${entry.id} ${entry.root} ${entry.apps.join(" ")}`
				.toLowerCase()
				.includes(search.toLowerCase()),
	);

	return (
		<ProductDemoFrame source="packages/ui/components/settings/connections/process-graph.tsx#ProcessCasesCard">
			<ProductCard className="overflow-hidden">
				<div className="space-y-3 border-b p-4">
					<div>
						<h3 className="flex items-center gap-2 font-semibold">
							<Workflow className="size-4" /> Process Cases
						</h3>
						<p className="mt-1 text-sm text-muted-foreground">
							End-to-end cases reconstructed across apps and events from the run
							correlation spine.
						</p>
					</div>
					<div className="flex flex-wrap items-center gap-2">
						<div className="flex items-center gap-1">
							{(["all", "Failed", "Running", "Completed"] as const).map(
								(item) => (
									<ProductButton
										key={item}
										variant={filter === item ? "secondary" : "ghost"}
										className="h-7 gap-1.5 px-2.5 text-xs"
										onClick={() => setFilter(item)}
									>
										{item === "all" ? "All" : item}
										<span className="tabular-nums text-muted-foreground">
											{item === "all"
												? processCases.length
												: processCases.filter((entry) => entry.status === item)
														.length}
										</span>
									</ProductButton>
								),
							)}
						</div>
						<label className="relative ml-auto">
							<Search className="absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
							<input
								value={search}
								onChange={(event) => setSearch(event.target.value)}
								placeholder="Filter by key, event, or app"
								className="h-8 w-60 rounded-md border bg-background pl-7 pr-3 text-xs"
							/>
						</label>
					</div>
				</div>
				<div className="space-y-2 p-4">
					{visible.map((processCase) => {
						const isExpanded = expanded === processCase.id;
						return (
							<div
								key={processCase.id}
								className={cn(
									"space-y-2 rounded-md border p-3 transition-colors hover:bg-muted/40",
									isExpanded && "bg-muted/30",
								)}
							>
								<div className="flex items-center justify-between gap-2">
									<div className="flex min-w-0 items-center gap-2">
										<CaseStatus status={processCase.status} />
										<span className="truncate text-sm font-medium">
											{processCase.root}
										</span>
										<span className="truncate text-xs text-muted-foreground">
											· {processCase.event}
										</span>
									</div>
									<div className="flex shrink-0 items-center gap-1">
										<span className="text-xs text-muted-foreground">
											moments ago
										</span>
										<ProductButton
											variant="ghost"
											className="size-6 p-0"
											ariaLabel={
												isExpanded ? "Collapse case" : "Inspect case timeline"
											}
											onClick={() =>
												setExpanded(isExpanded ? null : processCase.id)
											}
										>
											<ChevronRight
												className={cn(
													"size-3.5 transition-transform",
													isExpanded && "rotate-90",
												)}
											/>
										</ProductButton>
									</div>
								</div>
								<div className="flex min-w-0 flex-wrap items-center gap-1 text-xs text-muted-foreground">
									{processCase.apps.map((app, appIndex) => (
										<span key={app} className="flex items-center gap-1">
											{appIndex > 0 ? <ArrowRight className="size-3" /> : null}
											<span>{app}</span>
										</span>
									))}
								</div>
								<div className="flex flex-wrap gap-1">
									<ProductBadge
										variant="secondary"
										className="gap-1 font-normal"
									>
										<span className="text-muted-foreground">order_id</span>
										{processCase.id}
									</ProductBadge>
									<ProductBadge
										variant="secondary"
										className="gap-1 font-normal"
									>
										<span className="text-muted-foreground">customer_id</span>
										cust-481
									</ProductBadge>
								</div>
								<div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
									<ProductBadge variant="secondary">
										{processCase.runs} runs
									</ProductBadge>
									{processCase.failed > 0 ? (
										<ProductBadge variant="destructive">
											{processCase.failed} failed
										</ProductBadge>
									) : null}
									<span className="flex items-center gap-1">
										<Clock className="size-3" />
										{processCase.duration}
									</span>
								</div>
								{isExpanded ? (
									<div className="space-y-1 border-t pt-2">
										{[
											{
												id: "run-1",
												label: "Invoice Intake · OCR",
												left: 0,
												width: 18,
												duration: "420ms",
												failed: false,
											},
											{
												id: "run-2",
												label: "Risk Engine · Policy check",
												left: 15,
												width: 31,
												duration: "1.2s",
												failed: processCase.status === "Failed",
											},
											{
												id: "run-3",
												label: "ERP Sync · Create order",
												left: 44,
												width: 24,
												duration: "860ms",
												failed: false,
											},
											{
												id: "run-4",
												label: "Fulfilment · Dispatch",
												left: 65,
												width: 35,
												duration: "2.3s",
												failed: false,
											},
										].map((run, runIndex) => (
											<div
												key={run.id}
												className="flex items-center gap-2 text-xs"
											>
												<span
													className="w-44 shrink-0 truncate text-muted-foreground"
													style={{ paddingLeft: `${runIndex * 10}px` }}
												>
													{run.label}
												</span>
												<div className="relative h-2.5 min-w-0 flex-1 overflow-hidden rounded bg-muted/50">
													<span
														className={cn(
															"absolute inset-y-0 rounded",
															run.failed ? "bg-destructive" : "bg-primary/60",
														)}
														style={{
															left: `${run.left}%`,
															width: `${run.width}%`,
														}}
													/>
												</div>
												<span className="w-14 shrink-0 text-right tabular-nums text-muted-foreground">
													{run.duration}
												</span>
											</div>
										))}
									</div>
								) : null}
							</div>
						);
					})}
					{visible.length === 0 ? (
						<p className="py-6 text-center text-sm text-muted-foreground">
							No cases match the current filter.
						</p>
					) : null}
				</div>
			</ProductCard>
		</ProductDemoFrame>
	);
}

type SuiteConsoleTab = "branding" | "apps" | "visibility" | "danger";

const suiteMembers = [
	{
		id: "operations",
		name: "Operations Hub",
		kind: "Anchor",
		status: "Active",
		tone: "violet" as const,
	},
	{
		id: "reporting",
		name: "Reporting",
		kind: "Member",
		status: "Active",
		tone: "cyan" as const,
	},
	{
		id: "portal",
		name: "Partner Portal",
		kind: "Member",
		status: "Pending",
		tone: "amber" as const,
	},
];

function SuiteHeader() {
	return (
		<div className="flex items-start gap-3 border-b p-4">
			<div className="grid size-12 shrink-0 grid-cols-2 grid-rows-2 gap-px overflow-hidden rounded-xl ring-1 ring-border/50">
				<span className="bg-violet-500" />
				<span className="bg-cyan-500" />
				<span className="bg-amber-500" />
				<span className="bg-emerald-500" />
			</div>
			<div className="min-w-0 flex-1">
				<h3 className="truncate font-semibold">Operations Cloud</h3>
				<p className="text-sm text-muted-foreground">
					A suite is a visual collection only — it never grants runtime
					permissions, and every member app can leave at any time.
				</p>
			</div>
			<ProductBadge variant="secondary" className="shrink-0 gap-1 text-[11px]">
				<span className="size-2 rounded-full bg-slate-500" /> Private
			</ProductBadge>
		</div>
	);
}

function SuiteBrandingTab() {
	return (
		<div className="space-y-6">
			<div className="space-y-0.5">
				<h4 className="text-sm font-semibold">Identity</h4>
				<p className="text-xs text-muted-foreground">
					How the suite reads in the store and in invitations.
				</p>
			</div>
			<div className="grid gap-4 sm:grid-cols-2">
				<div className="space-y-1.5">
					<p className="text-sm font-medium">Icon</p>
					<button
						type="button"
						className="relative aspect-square w-full max-w-40 overflow-hidden rounded-xl border-2 border-dashed bg-gradient-to-br from-violet-500/20 to-cyan-500/20"
					>
						<span className="absolute inset-0 flex flex-col items-center justify-center gap-1 bg-background/40">
							<Layers className="size-5" />
							<span className="text-[11px] font-medium">
								Drop or click to replace
							</span>
						</span>
					</button>
					<p className="text-[11px] text-muted-foreground">
						Square, 1:1. PNG, JPG or WebP.
					</p>
				</div>
				<div className="space-y-1.5">
					<p className="text-sm font-medium">Banner</p>
					<button
						type="button"
						className="relative aspect-2/1 w-full overflow-hidden rounded-xl border-2 border-dashed bg-gradient-to-r from-violet-500/20 via-cyan-500/15 to-amber-500/20"
					>
						<span className="absolute inset-0 flex flex-col items-center justify-center gap-1 bg-background/30">
							<Layers3 className="size-5" />
							<span className="text-[11px] font-medium">
								Drop or click to replace
							</span>
						</span>
					</button>
					<p className="text-[11px] text-muted-foreground">
						Wide, 2:1. Shown behind the suite header.
					</p>
				</div>
			</div>
			<div className="grid gap-4 sm:grid-cols-2">
				<label className="space-y-1.5 text-sm font-medium">
					Name
					<input
						defaultValue="Operations Cloud"
						className="block h-10 w-full rounded-md border bg-background px-3 font-normal"
					/>
				</label>
				<label className="space-y-1.5 text-sm font-medium">
					Suite label
					<input
						defaultValue="Back-office platform"
						className="block h-10 w-full rounded-md border bg-background px-3 font-normal"
					/>
				</label>
			</div>
			<label className="block space-y-1.5 text-sm font-medium">
				Description
				<textarea
					defaultValue="Intake, policy, reporting, and partner operations in one governed platform."
					rows={3}
					className="block w-full resize-none rounded-md border bg-background p-3 font-normal"
				/>
			</label>
			<div className="flex flex-wrap gap-2">
				<ProductBadge variant="secondary">operations</ProductBadge>
				<ProductBadge variant="secondary">automation</ProductBadge>
				<ProductBadge variant="secondary">reporting</ProductBadge>
			</div>
			<ProductButton>Save changes</ProductButton>
		</div>
	);
}

function SuiteAppsTab() {
	const [added, setAdded] = useState(false);
	const members = added
		? [
				...suiteMembers,
				{
					id: "knowledge",
					name: "Knowledge Base",
					kind: "Member",
					status: "Pending",
					tone: "emerald" as const,
				},
			]
		: suiteMembers;
	return (
		<div className="space-y-6">
			<div className="space-y-0.5">
				<h4 className="text-sm font-semibold">
					{members.length} apps in this suite
				</h4>
				<p className="text-xs text-muted-foreground">
					Membership is presentation only. Each app keeps its own team,
					permissions and visibility.
				</p>
			</div>
			<div className="space-y-2">
				{members.map((member) => (
					<div key={member.id} className="rounded-lg border bg-card p-3">
						<div className="flex items-center gap-2.5">
							<ProductAvatar
								label={member.name.slice(0, 2).toUpperCase()}
								tone={member.tone}
								className="size-8 rounded-md text-[10px]"
							/>
							<div className="min-w-0 flex-1">
								<p className="truncate text-sm font-medium">{member.name}</p>
								<p className="truncate text-xs text-muted-foreground">
									Focused app in Operations Cloud
								</p>
							</div>
							{member.kind === "Anchor" ? (
								<ProductBadge variant="outline" className="text-[10px]">
									Anchor
								</ProductBadge>
							) : null}
							{member.status === "Pending" ? (
								<ProductBadge variant="secondary" className="text-[10px]">
									Pending
								</ProductBadge>
							) : null}
							{member.kind !== "Anchor" ? (
								<ProductButton
									variant="ghost"
									className="size-7 p-0"
									ariaLabel={`Remove ${member.name}`}
								>
									<X className="size-3.5" />
								</ProductButton>
							) : null}
						</div>
					</div>
				))}
			</div>
			<div className="space-y-3 border-t pt-4">
				<div>
					<h4 className="text-sm font-semibold">Add an app</h4>
					<p className="text-xs text-muted-foreground">
						Connected apps join instantly; everyone else receives an invite to
						accept.
					</p>
				</div>
				<div className="relative">
					<Search className="absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
					<input
						placeholder="Search your apps…"
						className="h-9 w-full rounded-md border bg-background pl-8 pr-3 text-sm"
					/>
				</div>
				<div className="rounded-lg border p-2.5">
					<div className="flex items-center gap-2.5">
						<ProductAvatar
							label="KB"
							tone="emerald"
							className="size-7 rounded-md text-[10px]"
						/>
						<div className="min-w-0 flex-1">
							<p className="truncate text-sm">Knowledge Base</p>
							<p className="truncate font-mono text-[10px] text-muted-foreground">
								app_knowledge
							</p>
						</div>
						<ProductButton
							variant="secondary"
							className="h-8"
							disabled={added}
							onClick={() => setAdded(true)}
						>
							<Plus className="size-3.5" />
							{added ? "Invited" : "Add"}
						</ProductButton>
					</div>
				</div>
			</div>
		</div>
	);
}

function SuiteVisibilityTab() {
	return (
		<div className="space-y-4">
			<ProductCard>
				<div className="border-b p-5">
					<h4 className="flex items-center gap-2 font-semibold">
						<Eye className="size-5" /> Visibility Status
					</h4>
					<p className="mt-1 text-sm text-muted-foreground">
						Control who can access your suite and how it's shared.
					</p>
				</div>
				<div className="space-y-4 p-5">
					<div className="flex items-center gap-3 rounded-lg border bg-muted p-4">
						<span className="size-3 rounded-full bg-slate-500" />
						<div>
							<p className="font-medium">Current: Private</p>
							<p className="text-sm text-muted-foreground">
								Only the anchor app's owners can manage this suite.
							</p>
						</div>
					</div>
					<div className="space-y-2">
						<p className="text-sm font-medium text-muted-foreground">
							Available transitions:
						</p>
						<button
							type="button"
							className="group flex w-full items-center justify-between rounded-md border p-3 text-left transition-colors hover:bg-muted/50"
						>
							<span className="flex items-center gap-3">
								<span className="size-3 rounded-full bg-emerald-500" />
								<span>
									<span className="block font-medium">Organization</span>
									<span className="block text-xs text-muted-foreground">
										Visible to people in your organization.
									</span>
								</span>
							</span>
							<ArrowRight className="size-4 opacity-0 transition-opacity group-hover:opacity-100" />
						</button>
					</div>
				</div>
			</ProductCard>
			<div className="flex items-start gap-2 rounded-lg border p-3 text-xs text-muted-foreground">
				<Shield className="mt-0.5 size-3.5 shrink-0" />
				<p>
					Public transitions require central review. Member app visibility is
					never changed by the suite.
				</p>
			</div>
		</div>
	);
}

function SuiteDangerTab() {
	return (
		<div className="space-y-6">
			<div className="space-y-2">
				<div>
					<h4 className="text-sm font-semibold">Lifecycle</h4>
					<p className="text-xs text-muted-foreground">
						Hide or retire the suite without changing its member apps.
					</p>
				</div>
				<select
					defaultValue="ACTIVE"
					className="h-10 w-full rounded-md border bg-background px-3 sm:w-96"
				>
					<option value="ACTIVE">
						Active — visible wherever the suite is shared
					</option>
					<option value="INACTIVE">Inactive — hidden, keeps its members</option>
					<option value="ARCHIVED">Archived — retired, read-only</option>
				</select>
				<p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
					<Archive className="size-3" /> Member apps keep their own lifecycle.
				</p>
			</div>
			<div className="space-y-2 border-t pt-4">
				<h4 className="text-sm font-semibold">Delete suite</h4>
				<p className="text-xs text-muted-foreground">
					Delete the suite identity and curation. No member app is deleted.
				</p>
				<ProductButton variant="destructive">
					<Trash2 className="size-3.5" /> Delete suite
				</ProductButton>
			</div>
		</div>
	);
}

export function SuiteConsoleDemo() {
	const [tab, setTab] = useState<SuiteConsoleTab>("branding");
	const tabs: Array<{ id: SuiteConsoleTab; label: string }> = [
		{ id: "branding", label: "Branding" },
		{ id: "apps", label: "Apps" },
		{ id: "visibility", label: "Visibility" },
		{ id: "danger", label: "Danger zone" },
	];
	return (
		<ProductDemoFrame source="packages/ui/components/settings/team/group-console.tsx">
			<div className="mx-auto flex w-full max-w-3xl flex-col gap-0 overflow-hidden rounded-xl border bg-background shadow-sm">
				<SuiteHeader />
				<div className="mx-4 mt-4 flex w-fit max-w-[calc(100%-2rem)] gap-1 overflow-x-auto rounded-lg bg-muted p-1">
					{tabs.map((item) => (
						<button
							key={item.id}
							type="button"
							onClick={() => setTab(item.id)}
							className={cn(
								"shrink-0 rounded-md px-3 py-1.5 text-sm font-medium",
								tab === item.id
									? "bg-background shadow-sm"
									: "text-muted-foreground hover:text-foreground",
							)}
						>
							{item.label}
						</button>
					))}
				</div>
				<div className="p-4 pb-10">
					{tab === "branding" ? (
						<SuiteBrandingTab />
					) : tab === "apps" ? (
						<SuiteAppsTab />
					) : tab === "visibility" ? (
						<SuiteVisibilityTab />
					) : (
						<SuiteDangerTab />
					)}
				</div>
			</div>
		</ProductDemoFrame>
	);
}

const libraryApps = [
	{
		id: "operations",
		name: "Operations Hub",
		description: "Work queue and approvals",
		tone: "violet" as const,
	},
	{
		id: "reporting",
		name: "Reporting",
		description: "Operational intelligence",
		tone: "cyan" as const,
	},
	{
		id: "portal",
		name: "Partner Portal",
		description: "External collaboration",
		tone: "amber" as const,
	},
	{
		id: "knowledge",
		name: "Knowledge Base",
		description: "Runbooks and policies",
		tone: "emerald" as const,
	},
];

function SuiteGlyph() {
	return (
		<div className="grid size-11 shrink-0 grid-cols-2 grid-rows-2 gap-px overflow-hidden rounded-xl bg-border/40 ring-1 ring-border/50">
			<span className="bg-violet-500" />
			<span className="bg-cyan-500" />
			<span className="bg-amber-500" />
			<span className="bg-emerald-500" />
		</div>
	);
}

export function SuiteLibraryDemo() {
	const [expanded, setExpanded] = useState(true);
	return (
		<ProductDemoFrame source="packages/ui/components/library/library-suite-shelf.tsx">
			<section>
				<div className="mb-3 flex items-center gap-2">
					<Layers className="size-3.5 text-muted-foreground/50" />
					<h3 className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
						Suites
					</h3>
					<span className="text-xs text-muted-foreground/30">1</span>
				</div>
				<div
					className={cn(
						"group/suite relative overflow-hidden rounded-xl border border-border/40 bg-card/80 backdrop-blur-sm transition-all duration-300",
						expanded
							? "border-primary/20 bg-card/95"
							: "hover:border-primary/20",
					)}
				>
					<div className="pointer-events-none absolute bottom-0 left-0 top-0 w-64 overflow-hidden bg-gradient-to-r from-violet-500/25 via-cyan-500/15 to-transparent opacity-40 transition-opacity duration-300 group-hover/suite:opacity-60">
						<div className="absolute inset-0 bg-linear-to-r from-transparent to-card" />
					</div>
					<button
						type="button"
						onClick={() => setExpanded((value) => !value)}
						aria-expanded={expanded}
						className="relative z-10 flex w-full cursor-pointer items-center gap-3 px-3 py-2.5 text-left"
					>
						<SuiteGlyph />
						<div className="min-w-0 flex-1">
							<div className="flex items-center gap-2">
								<h4 className="truncate text-sm font-semibold">
									Operations Cloud
								</h4>
								<span className="inline-flex shrink-0 items-center gap-1 rounded-full border border-border/50 bg-background/60 px-1.5 py-px text-[10px] font-medium text-muted-foreground">
									<EyeOff className="size-2.5" /> Private
								</span>
							</div>
							<p className="mt-0.5 truncate text-xs text-muted-foreground/80">
								Back-office platform
							</p>
						</div>
						<div className="flex shrink-0 items-center gap-2.5 text-muted-foreground/70">
							<span className="text-xs font-medium tabular-nums">4 apps</span>
							<ChevronDown
								className={cn(
									"size-4 transition-transform duration-300",
									expanded && "rotate-180",
								)}
							/>
						</div>
					</button>
					{expanded ? (
						<div className="relative z-10 border-t border-border/40 bg-background/30 px-3 py-3">
							<div className="grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(180px,1fr))]">
								{libraryApps.map((app) => (
									<button
										key={app.id}
										type="button"
										className="group rounded-xl border border-border/60 bg-card p-3 text-left transition-all hover:-translate-y-0.5 hover:border-primary/30 hover:shadow-md"
									>
										<ProductAvatar
											label={app.name.slice(0, 2).toUpperCase()}
											tone={app.tone}
										/>
										<h5 className="mt-4 text-sm font-medium">{app.name}</h5>
										<p className="mt-1 text-xs text-muted-foreground">
											{app.description}
										</p>
										<span className="mt-4 flex items-center gap-1 text-[11px] font-medium text-primary">
											Open app{" "}
											<ArrowRight className="size-3 transition-transform group-hover:translate-x-0.5" />
										</span>
									</button>
								))}
							</div>
						</div>
					) : null}
				</div>
			</section>
		</ProductDemoFrame>
	);
}

function MemberReadinessList() {
	const readiness = [
		{ id: "operations", name: "Operations Hub", status: "SUBMITTED" },
		{ id: "reporting", name: "Reporting", status: "APPROVED" },
		{ id: "portal", name: "Partner Portal", status: "SUBMITTED" },
	];
	return (
		<div className="space-y-2">
			<div className="space-y-0.5">
				<h4 className="text-sm font-semibold">EU AI Act readiness</h4>
				<p className="text-xs text-muted-foreground">
					Every active member app needs a submitted, non-blocked assessment
					before the suite can be published.
				</p>
			</div>
			<div className="divide-y rounded-lg border">
				{readiness.map((entry) => (
					<div key={entry.id} className="flex items-center gap-2 p-2.5">
						<CheckCircle2 className="size-4 shrink-0 text-emerald-500" />
						<span className="flex-1 truncate font-mono text-xs">
							{entry.name}
						</span>
						<ProductBadge variant="secondary" className="text-[10px]">
							{entry.status}
						</ProductBadge>
					</div>
				))}
			</div>
		</div>
	);
}

export function SuitePublicationDemo() {
	const [confirming, setConfirming] = useState(false);
	const [submitted, setSubmitted] = useState(false);
	return (
		<ProductDemoFrame source="packages/ui/components/settings/team/group-console.tsx#VisibilityTab">
			<div className="relative mx-auto max-w-3xl space-y-4 rounded-xl border bg-background p-4 sm:p-6">
				{submitted ? (
					<div className="space-y-2 rounded-lg border border-primary/40 bg-primary/5 p-3">
						<div className="flex items-center gap-2">
							<Clock className="size-4 text-primary" />
							<p className="text-sm font-medium">Submitted for review</p>
							<ProductBadge variant="secondary" className="ml-auto text-[10px]">
								PENDING
							</ProductBadge>
						</div>
						<p className="text-xs text-muted-foreground">
							Target visibility Public · submitted moments ago
						</p>
					</div>
				) : null}
				<ProductCard>
					<div className="border-b p-5">
						<h3 className="flex items-center gap-2 font-semibold">
							<Eye className="size-5" /> Visibility Status
						</h3>
						<p className="mt-1 text-sm text-muted-foreground">
							Control who can access your suite and how it's shared.
						</p>
					</div>
					<div className="space-y-4 p-5">
						<div className="flex items-center gap-3 rounded-lg border bg-muted p-4">
							<span className="size-3 rounded-full bg-slate-500" />
							<div>
								<p className="font-medium">Current: Private</p>
								<p className="text-sm text-muted-foreground">
									Only invited people can discover this suite.
								</p>
							</div>
						</div>
						<div className="space-y-3">
							<p className="text-sm font-medium text-muted-foreground">
								Available transitions:
							</p>
							<button
								type="button"
								disabled={submitted}
								onClick={() => setConfirming(true)}
								className="group flex h-fit w-full items-center justify-between rounded-md border p-3 text-left transition-colors hover:bg-muted/50 disabled:opacity-50"
							>
								<span className="flex items-center gap-3">
									<span className="size-3 rounded-full bg-emerald-500" />
									<span>
										<span className="block font-medium">Public</span>
										<span className="block text-xs text-muted-foreground">
											Listed in Explore after central review.
										</span>
									</span>
								</span>
								<ArrowRight className="size-4 opacity-0 transition-opacity group-hover:opacity-100" />
							</button>
						</div>
						<div className="space-y-1 border-t pt-3 text-xs text-muted-foreground">
							<p className="flex items-center gap-1">
								<Users className="size-3" /> Public transitions require central
								review (1–3 days)
							</p>
						</div>
					</div>
				</ProductCard>
				<MemberReadinessList />
				{confirming ? (
					<div className="absolute inset-0 z-20 flex items-center justify-center rounded-xl bg-background/75 p-4 backdrop-blur-sm">
						<dialog
							open
							className="w-full max-w-md rounded-xl border bg-background p-5 shadow-xl"
						>
							<div className="flex items-center gap-3">
								<span className="rounded-full bg-blue-50 p-2 dark:bg-blue-950">
									<ShieldCheck className="size-5 text-blue-500" />
								</span>
								<h3 className="font-semibold">Submit suite for publication?</h3>
							</div>
							<p className="mt-3 text-sm text-muted-foreground">
								Operations Cloud stays Private while the branding and all active
								member assessments are reviewed.
							</p>
							<div className="my-4 flex items-center justify-center gap-2 rounded-lg bg-muted p-3 text-sm font-medium">
								<span className="size-2 rounded-full bg-slate-500" /> Private{" "}
								<ArrowRight className="size-4 text-muted-foreground" />
								<span className="size-2 rounded-full bg-emerald-500" /> Public
							</div>
							<div className="flex justify-end gap-2">
								<ProductButton
									variant="outline"
									onClick={() => setConfirming(false)}
								>
									Cancel
								</ProductButton>
								<ProductButton
									onClick={() => {
										setConfirming(false);
										setSubmitted(true);
									}}
								>
									Change Visibility
								</ProductButton>
							</div>
						</dialog>
					</div>
				) : null}
			</div>
		</ProductDemoFrame>
	);
}

type StudioTab =
	| "overview"
	| "objects"
	| "model"
	| "actions"
	| "sharing"
	| "sources"
	| "queries";

const studioTabs: Array<{
	id: StudioTab;
	label: string;
	icon: typeof Layers3;
}> = [
	{ id: "overview", label: "Overview", icon: LayoutDashboard },
	{ id: "objects", label: "Explore", icon: Box },
	{ id: "model", label: "Model", icon: Network },
	{ id: "actions", label: "Actions", icon: Workflow },
	{ id: "sharing", label: "Sharing", icon: Share2 },
	{ id: "sources", label: "Sources", icon: Database },
	{ id: "queries", label: "Queries", icon: SquareTerminal },
];

function DataStudioOverviewPanel({
	onNavigate,
}: Readonly<{ onNavigate: (view: StudioTab) => void }>) {
	const metrics = [
		{ label: "Ontologies", value: "3", icon: Layers3 },
		{ label: "Object types", value: "8", icon: Box },
		{ label: "Actions", value: "5", icon: Workflow },
		{ label: "Shared", value: "1", icon: Share2 },
		{ label: "Remote", value: "2", icon: Cloud },
	];
	return (
		<div className="space-y-6">
			<div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
				{metrics.map(({ label, value, icon: Icon }) => (
					<ProductCard key={label}>
						<div className="flex items-center justify-between p-4">
							<div>
								<p className="text-xs font-medium text-muted-foreground">
									{label}
								</p>
								<p className="mt-1 text-2xl font-semibold">{value}</p>
							</div>
							<div className="rounded-xl bg-primary/10 p-2.5 text-primary">
								<Icon className="size-4" />
							</div>
						</div>
					</ProductCard>
				))}
			</div>
			<div className="grid gap-5 xl:grid-cols-[1.4fr_1fr]">
				<ProductCard>
					<div className="flex items-center justify-between border-b p-5">
						<div>
							<h3 className="font-semibold">Your semantic layer</h3>
							<p className="mt-1 text-sm text-muted-foreground">
								Objects, relationships, views, and operations over 12 tables.
							</p>
						</div>
						<ProductButton className="h-8">
							<Plus className="size-4" /> New ontology
						</ProductButton>
					</div>
					<div className="space-y-2 p-5">
						{[
							{
								id: "orders",
								name: "Order Operations",
								objects: 4,
								edges: 5,
								bindings: true,
							},
							{
								id: "suppliers",
								name: "Supplier Network",
								objects: 3,
								edges: 4,
								bindings: true,
							},
							{
								id: "customers",
								name: "Customer 360",
								objects: 1,
								edges: 2,
								bindings: false,
							},
						].map((ontology) => (
							<button
								key={ontology.id}
								type="button"
								onClick={() => onNavigate("model")}
								className="flex w-full items-center gap-3 rounded-lg border p-3 text-left transition-colors hover:bg-muted/50"
							>
								<div className="rounded-lg bg-primary/10 p-2 text-primary">
									<Network className="size-4" />
								</div>
								<div className="min-w-0 flex-1">
									<p className="truncate text-sm font-medium">
										{ontology.name}
									</p>
									<p className="text-xs text-muted-foreground">
										{ontology.objects} objects · {ontology.edges} relationships
									</p>
								</div>
								{ontology.bindings ? (
									<ProductBadge variant="secondary">Bindings</ProductBadge>
								) : null}
								<ChevronRight className="size-4 text-muted-foreground" />
							</button>
						))}
					</div>
				</ProductCard>
				<ProductCard>
					<div className="border-b p-5">
						<h3 className="font-semibold">Start with a task</h3>
					</div>
					<div className="space-y-2 p-5">
						{[
							{
								id: "objects" as const,
								title: "Explore business objects",
								description: "Search and inspect generated object views",
								icon: Search,
							},
							{
								id: "model" as const,
								title: "Shape the model",
								description: "Review types, links, mappings, and health",
								icon: GitBranch,
							},
							{
								id: "actions" as const,
								title: "Connect an action",
								description: "Bind an operation to a typed board entry",
								icon: Workflow,
							},
							{
								id: "sharing" as const,
								title: "Expose a contract",
								description: "Share with projects through app connections",
								icon: Share2,
							},
						].map(({ id, title, description, icon: Icon }) => (
							<button
								key={id}
								type="button"
								onClick={() => onNavigate(id)}
								className="flex w-full items-center gap-3 rounded-lg p-2.5 text-left transition-colors hover:bg-muted"
							>
								<Icon className="size-4 text-muted-foreground" />
								<div className="min-w-0 flex-1">
									<p className="text-sm font-medium">{title}</p>
									<p className="truncate text-xs text-muted-foreground">
										{description}
									</p>
								</div>
								<ArrowRight className="size-3.5 text-muted-foreground" />
							</button>
						))}
					</div>
				</ProductCard>
			</div>
		</div>
	);
}

const objectRows = [
	{
		id: "ORD-1048",
		status: "Ready",
		customer: "Northstar GmbH",
		total: "€12,480",
	},
	{
		id: "ORD-1047",
		status: "Review",
		customer: "Acme Industries",
		total: "€8,240",
	},
	{ id: "ORD-1046", status: "Ready", customer: "Kite GmbH", total: "€5,910" },
];

function DataStudioExplorePanel() {
	const [objectType, setObjectType] = useState("Order");
	const [query, setQuery] = useState("");
	const [selectedRow, setSelectedRow] = useState<
		(typeof objectRows)[number] | null
	>(null);
	const rows = objectRows.filter((row) =>
		Object.values(row).some((value) =>
			value.toLowerCase().includes(query.toLowerCase()),
		),
	);
	return (
		<div className="relative grid h-[31rem] min-h-0 grid-cols-1 overflow-hidden rounded-xl border lg:grid-cols-[260px_minmax(0,1fr)]">
			<aside className="min-h-0 border-b bg-muted/20 lg:border-b-0 lg:border-r">
				<div className="border-b p-3">
					<button
						type="button"
						className="flex h-9 w-full items-center justify-between rounded-md border bg-background px-3 text-sm"
					>
						<span>Order Operations</span>
						<ChevronDown className="size-4" />
					</button>
				</div>
				<div className="space-y-1 p-2">
					<p className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
						Object types
					</p>
					{["Order", "Customer", "Line Item", "Warehouse"].map((item) => (
						<button
							key={item}
							type="button"
							onClick={() => setObjectType(item)}
							className={cn(
								"flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-sm",
								objectType === item
									? "bg-primary text-primary-foreground"
									: "hover:bg-muted",
							)}
						>
							<CircleDot className="size-3.5" />
							<span className="truncate">{item}</span>
						</button>
					))}
				</div>
			</aside>
			<section className="flex min-h-0 min-w-0 flex-col">
				<header className="flex flex-col gap-3 border-b p-4 sm:flex-row sm:items-center sm:justify-between">
					<div>
						<div className="flex items-center gap-2">
							<h3 className="font-semibold">{objectType}</h3>
							<ProductBadge>3 preview</ProductBadge>
						</div>
						<p className="text-xs text-muted-foreground">
							Standard object view · source orders
						</p>
					</div>
					<div className="flex items-center gap-2">
						<label className="relative min-w-56">
							<Search className="absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
							<input
								value={query}
								onChange={(event) => setQuery(event.target.value)}
								placeholder="Filter loaded objects"
								className="h-9 w-full rounded-md border bg-background pl-8 pr-3 text-sm"
							/>
						</label>
						<ProductButton
							variant="outline"
							className="size-9 p-0"
							ariaLabel="Refresh objects"
						>
							<RefreshCw className="size-4" />
						</ProductButton>
					</div>
				</header>
				<div className="min-h-0 flex-1 overflow-auto">
					<table className="w-full text-sm">
						<thead className="sticky top-0 z-10 bg-background">
							<tr className="border-b">
								<th className="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground">
									Order ID
								</th>
								<th className="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground">
									Status
								</th>
								<th className="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground">
									Customer
								</th>
								<th className="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground">
									Total
								</th>
								<th className="w-10">
									<span className="sr-only">Open object</span>
								</th>
							</tr>
						</thead>
						<tbody>
							{rows.map((row) => (
								<tr
									key={row.id}
									className="border-b transition-colors hover:bg-muted/50"
								>
									<td className="px-4 py-2.5 font-medium">{row.id}</td>
									<td className="px-4 py-2.5">{row.status}</td>
									<td className="px-4 py-2.5">{row.customer}</td>
									<td className="px-4 py-2.5">{row.total}</td>
									<td className="pr-3">
										<ProductButton
											variant="ghost"
											className="size-8 p-0"
											ariaLabel={`Open ${row.id}`}
											onClick={() => setSelectedRow(row)}
										>
											<ChevronRight className="size-4 text-muted-foreground" />
										</ProductButton>
									</td>
								</tr>
							))}
						</tbody>
					</table>
				</div>
			</section>
			{selectedRow ? (
				<aside className="absolute inset-y-0 right-0 z-20 w-full border-l bg-background shadow-xl sm:max-w-sm">
					<div className="flex items-start gap-3 border-b p-5 pr-12">
						<div className="flex size-11 items-center justify-center rounded-xl bg-primary text-primary-foreground">
							<Box className="size-5" />
						</div>
						<div>
							<p className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
								Order
							</p>
							<h3 className="text-xl font-semibold">{selectedRow.id}</h3>
						</div>
						<ProductButton
							variant="ghost"
							className="absolute right-3 top-3 size-8 p-0"
							ariaLabel="Close object"
							onClick={() => setSelectedRow(null)}
						>
							<X className="size-4" />
						</ProductButton>
					</div>
					<div className="space-y-5 p-5">
						<div>
							<p className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
								Prominent properties
							</p>
							<div className="grid grid-cols-2 gap-2.5">
								<div className="rounded-xl border bg-muted/30 p-3">
									<p className="text-[10px] uppercase text-muted-foreground">
										Customer
									</p>
									<p className="mt-1 text-sm font-medium">
										{selectedRow.customer}
									</p>
								</div>
								<div className="rounded-xl border bg-muted/30 p-3">
									<p className="text-[10px] uppercase text-muted-foreground">
										Value
									</p>
									<p className="mt-1 text-sm font-medium">
										{selectedRow.total}
									</p>
								</div>
							</div>
						</div>
						<div>
							<p className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
								Available actions
							</p>
							<button
								type="button"
								className="flex w-full items-center gap-3 rounded-xl border bg-card px-3.5 py-2.5 text-left hover:border-primary/40"
							>
								<span className="flex size-8 items-center justify-center rounded-lg bg-primary/10 text-primary">
									<Workflow className="size-4" />
								</span>
								<span className="min-w-0 flex-1">
									<span className="block text-sm font-medium">
										Approve order
									</span>
									<span className="block text-xs text-muted-foreground">
										Governed board action
									</span>
								</span>
								<ArrowRight className="size-4 text-muted-foreground" />
							</button>
						</div>
					</div>
				</aside>
			) : null}
		</div>
	);
}

function DataStudioModelPanel() {
	return (
		<div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
			<div className="space-y-3">
				<div className="flex items-center justify-between">
					<div>
						<h3 className="font-semibold">Ontologies</h3>
						<p className="text-xs text-muted-foreground">
							Semantic overlays on native tables.
						</p>
					</div>
					<ProductButton className="h-8">
						<Plus className="size-4" /> New
					</ProductButton>
				</div>
				{["Order Operations", "Supplier Network", "Customer 360"].map(
					(name, index) => (
						<button
							key={name}
							type="button"
							className={cn(
								"relative w-full rounded-xl border p-4 text-left transition-colors",
								index === 0
									? "border-primary bg-primary/5"
									: "hover:bg-muted/40",
							)}
						>
							<p className="font-medium">{name}</p>
							<p className="mt-1 text-xs text-muted-foreground">
								Objects and relationships over live tables.
							</p>
							<p className="mt-3 text-xs text-muted-foreground">
								{4 - index} objects · {5 - index} links
							</p>
						</button>
					),
				)}
			</div>
			<div className="space-y-5 rounded-xl border p-5">
				<div className="flex items-start justify-between">
					<div>
						<div className="flex items-center gap-2">
							<h3 className="text-lg font-semibold">Order Operations</h3>
							<ProductBadge variant="default" className="gap-1">
								<Code2 className="size-3" /> Bindings
							</ProductBadge>
						</div>
						<p className="mt-1 text-sm text-muted-foreground">
							Order lifecycle objects, relationships, and governed actions.
						</p>
					</div>
					<ProductButton variant="outline">
						<Network className="size-4" /> Explore data graph
					</ProductButton>
				</div>
				<div>
					<h4 className="text-sm font-medium">Object types</h4>
					<p className="text-xs text-muted-foreground">
						Standard object views generated from table mappings.
					</p>
					<div className="mt-3 grid gap-3 md:grid-cols-2">
						{["Order", "Customer", "Line Item", "Warehouse"].map(
							(object, index) => (
								<div key={object} className="rounded-xl border p-4">
									<div className="flex items-start gap-3">
										<span
											className={cn(
												"mt-1 size-3 rounded-full",
												[
													"bg-violet-500",
													"bg-cyan-500",
													"bg-amber-500",
													"bg-emerald-500",
												][index],
											)}
										/>
										<div className="min-w-0 flex-1">
											<div className="flex items-center justify-between gap-2">
												<p className="font-medium">{object}</p>
												<code className="text-[10px] text-muted-foreground">
													{object.toLowerCase().replace(" ", "_")}
												</code>
											</div>
											<p className="mt-1 text-xs text-muted-foreground">
												Mapped from {object.toLowerCase().replace(" ", "_")}{" "}
												table
											</p>
											<div className="mt-3 flex gap-1.5">
												<ProductBadge>id</ProductBadge>
												<ProductBadge>name</ProductBadge>
												<ProductBadge>status</ProductBadge>
											</div>
										</div>
									</div>
								</div>
							),
						)}
					</div>
				</div>
			</div>
		</div>
	);
}

function DataStudioActionsPanel() {
	return (
		<div className="space-y-5">
			<div className="flex items-center justify-between">
				<div>
					<h3 className="font-semibold">Ontology actions</h3>
					<p className="text-sm text-muted-foreground">
						Attach governed, version-pinned boards to typed objects.
					</p>
				</div>
				<ProductButton>
					<Plus className="size-4" /> Define action
				</ProductButton>
			</div>
			<div className="grid gap-3 lg:grid-cols-2">
				{[
					{ name: "Approve order", object: "Order", board: "Order approvals" },
					{
						name: "Dispatch shipment",
						object: "Shipment",
						board: "Fulfilment",
					},
				].map((action) => (
					<ProductCard key={action.name}>
						<div className="p-4">
							<div className="flex items-start justify-between">
								<div className="flex gap-3">
									<div className="rounded-lg bg-primary/10 p-2 text-primary">
										<Workflow className="size-4" />
									</div>
									<div>
										<p className="font-medium">{action.name}</p>
										<p className="text-xs text-muted-foreground">
											Action on {action.object}
										</p>
									</div>
								</div>
								<ProductBadge variant="secondary">Enabled</ProductBadge>
							</div>
							<p className="mt-3 text-sm text-muted-foreground">
								Runs a protected board action against the selected object.
							</p>
							<div className="mt-4 grid grid-cols-2 gap-2 text-xs">
								<div className="rounded-lg bg-muted/40 p-2">
									<span className="text-muted-foreground">Board</span>
									<p className="mt-0.5 font-medium">{action.board}</p>
								</div>
								<div className="rounded-lg bg-muted/40 p-2">
									<span className="text-muted-foreground">Binding</span>
									<p className="mt-0.5 font-mono text-[10px]">
										v1.6.0 · pinned
									</p>
								</div>
							</div>
						</div>
					</ProductCard>
				))}
			</div>
		</div>
	);
}

function DataStudioSourcesPanel() {
	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between">
				<div>
					<h3 className="font-semibold">Native tables</h3>
					<p className="text-sm text-muted-foreground">
						Open a source to inspect rows, schema, and indexes.
					</p>
				</div>
				<ProductButton>
					<Plus className="size-4" /> New table
				</ProductButton>
			</div>
			<label className="relative block max-w-xl">
				<Search className="absolute left-3 top-2.5 size-4 text-muted-foreground" />
				<input
					placeholder="Search tables"
					className="h-9 w-full rounded-md border bg-background pl-9 pr-3 text-sm"
				/>
			</label>
			<div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
				{[
					"orders",
					"customers",
					"line_items",
					"warehouses",
					"shipments",
					"suppliers",
				].map((table) => (
					<ProductCard
						key={table}
						className="cursor-pointer overflow-hidden transition-all hover:bg-muted/50 hover:shadow-lg"
					>
						<div className="space-y-5 p-5">
							<div className="flex items-center gap-3">
								<div className="rounded-xl bg-primary/10 p-2.5 text-primary">
									<Table2 className="size-5" />
								</div>
								<div>
									<h4 className="text-sm font-semibold">{table}</h4>
									<p className="text-xs text-muted-foreground">
										Project database
									</p>
								</div>
							</div>
							<div className="flex items-center justify-between border-t pt-3 text-xs text-muted-foreground">
								<span>8 columns</span>
								<span className="font-medium text-foreground">
									Open table →
								</span>
							</div>
						</div>
					</ProductCard>
				))}
			</div>
		</div>
	);
}

function DataStudioQueriesPanel() {
	const [ran, setRan] = useState(false);
	return (
		<div className="flex h-[31rem] min-h-0 overflow-hidden rounded-xl border">
			<aside className="hidden w-52 shrink-0 border-r bg-muted/20 p-3 sm:block">
				<div className="flex items-center justify-between">
					<p className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
						Saved queries
					</p>
					<Plus className="size-4" />
				</div>
				<button
					type="button"
					className="mt-3 w-full rounded-md bg-background p-2 text-left text-sm shadow-sm"
				>
					Open orders by value
				</button>
				<button
					type="button"
					className="mt-1 w-full rounded-md p-2 text-left text-sm text-muted-foreground hover:bg-muted"
				>
					Warehouse backlog
				</button>
			</aside>
			<section className="flex min-w-0 flex-1 flex-col">
				<div className="flex min-h-12 flex-wrap items-center gap-2 border-b bg-muted/20 px-3 py-1.5">
					<ProductButton variant="ghost" className="size-8 p-0">
						<PanelLeftClose className="size-4" />
					</ProductButton>
					<div className="flex items-center gap-0.5 rounded-lg bg-muted p-0.5">
						<button
							type="button"
							className="flex items-center gap-1.5 rounded-md bg-background px-2.5 py-1 text-xs font-medium shadow-sm"
						>
							<Database className="size-3.5" /> Native
						</button>
						<button
							type="button"
							className="flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium text-muted-foreground"
						>
							<Boxes className="size-3.5" /> Ontology
						</button>
						<button
							type="button"
							className="flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium text-muted-foreground"
						>
							<Cloud className="size-3.5" /> Remote
						</button>
					</div>
					<div className="ml-auto flex items-center gap-2">
						<ProductButton variant="outline" className="h-8">
							<Save className="size-4" /> Save
						</ProductButton>
						<ProductButton className="h-8" onClick={() => setRan(true)}>
							<Play className="size-4" /> Run{" "}
							<kbd className="hidden rounded border border-primary-foreground/30 px-1 text-[10px] sm:inline">
								⌘↵
							</kbd>
						</ProductButton>
					</div>
				</div>
				<div className="grid min-h-0 flex-1 grid-rows-[1fr_1fr]">
					<div className="min-h-0 border-b bg-slate-950 p-4 font-mono text-sm text-slate-100">
						<span className="text-violet-300">SELECT</span> order_id,
						customer_name, total
						<br />
						<span className="text-violet-300">FROM</span> orders
						<br />
						<span className="text-violet-300">WHERE</span> status ={" "}
						<span className="text-emerald-300">'READY'</span>
						<br />
						<span className="text-violet-300">ORDER BY</span> total DESC;
					</div>
					<div className="min-h-0 overflow-auto">
						{ran ? (
							<table className="w-full text-sm">
								<thead>
									<tr className="border-b bg-muted/30">
										<th className="p-2 text-left">order_id</th>
										<th className="p-2 text-left">customer_name</th>
										<th className="p-2 text-left">total</th>
									</tr>
								</thead>
								<tbody>
									{objectRows.map((row) => (
										<tr key={row.id} className="border-b">
											<td className="p-2">{row.id}</td>
											<td className="p-2">{row.customer}</td>
											<td className="p-2">{row.total}</td>
										</tr>
									))}
								</tbody>
							</table>
						) : (
							<div className="flex h-full items-center justify-center text-sm text-muted-foreground">
								Run the query to inspect results.
							</div>
						)}
					</div>
				</div>
			</section>
		</div>
	);
}

export function DataStudioTourDemo() {
	const [tab, setTab] = useState<StudioTab>("overview");
	return (
		<ProductDemoFrame source="packages/ui/components/settings/explore/explore-page.tsx#DatabaseOverview">
			<div className="flex flex-col overflow-hidden rounded-xl border bg-background">
				<div className="p-4 pb-0 sm:p-6 sm:pb-0">
					<header className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
						<div className="flex items-center gap-3">
							<div className="rounded-xl bg-primary/10 p-2.5 text-primary">
								<Layers3 className="size-5" />
							</div>
							<div>
								<h3 className="text-2xl font-semibold">Data Studio</h3>
								<p className="text-sm text-muted-foreground">
									Model, explore, operate, and share your project data.
								</p>
							</div>
						</div>
						<div className="flex items-center gap-2">
							<ProductButton variant="outline" className="h-8">
								<RefreshCw className="size-4" /> Refresh
							</ProductButton>
							<ProductButton className="h-8">
								<Plus className="size-4" /> Set up ontology
							</ProductButton>
						</div>
					</header>
				</div>
				<div className="overflow-x-auto px-4 pt-4 sm:px-6">
					<div className="flex w-max gap-1 rounded-lg bg-muted p-1">
						{studioTabs.map(({ id, label, icon: Icon }) => (
							<button
								key={id}
								type="button"
								onClick={() => setTab(id)}
								className={cn(
									"flex items-center rounded-md px-3 py-1.5 text-sm font-medium",
									tab === id
										? "bg-background shadow-sm"
										: "text-muted-foreground hover:text-foreground",
								)}
							>
								<Icon className="mr-1.5 size-3.5" />
								{label}
							</button>
						))}
					</div>
				</div>
				<div className="p-4 sm:p-6">
					{tab === "overview" ? (
						<DataStudioOverviewPanel onNavigate={setTab} />
					) : tab === "objects" ? (
						<DataStudioExplorePanel />
					) : tab === "model" ? (
						<DataStudioModelPanel />
					) : tab === "actions" ? (
						<DataStudioActionsPanel />
					) : tab === "sharing" ? (
						<OntologySharingSurface />
					) : tab === "sources" ? (
						<DataStudioSourcesPanel />
					) : (
						<DataStudioQueriesPanel />
					)}
				</div>
			</div>
		</ProductDemoFrame>
	);
}

type ViewerNode = {
	id: string;
	label: string;
	caption: string;
	x: number;
	y: number;
	color: string;
	degree: number;
	pagerank: number;
	component: number;
};

const viewerNodes: ViewerNode[] = [
	{
		id: "supplier",
		label: "Supplier",
		caption: "Northstar Supply",
		x: 13,
		y: 44,
		color: "bg-cyan-500",
		degree: 8,
		pagerank: 0.12,
		component: 0,
	},
	{
		id: "order",
		label: "Order",
		caption: "ORD-1048",
		x: 38,
		y: 20,
		color: "bg-violet-500",
		degree: 14,
		pagerank: 0.31,
		component: 0,
	},
	{
		id: "warehouse",
		label: "Warehouse",
		caption: "BER-2",
		x: 63,
		y: 52,
		color: "bg-amber-500",
		degree: 11,
		pagerank: 0.24,
		component: 0,
	},
	{
		id: "shipment",
		label: "Shipment",
		caption: "SHIP-8821",
		x: 84,
		y: 23,
		color: "bg-emerald-500",
		degree: 7,
		pagerank: 0.18,
		component: 0,
	},
	{
		id: "customer",
		label: "Customer",
		caption: "Kite GmbH",
		x: 35,
		y: 76,
		color: "bg-blue-500",
		degree: 5,
		pagerank: 0.09,
		component: 1,
	},
];

export function GraphAnalyticsDemo() {
	const [query, setQuery] = useState("");
	const [limit, setLimit] = useState("200");
	const [selectedId, setSelectedId] = useState("order");
	const selected =
		viewerNodes.find((node) => node.id === selectedId) ?? viewerNodes[0];
	const matches = useMemo(
		() =>
			new Set(
				viewerNodes
					.filter((node) =>
						`${node.label} ${node.caption}`
							.toLowerCase()
							.includes(query.toLowerCase()),
					)
					.map((node) => node.id),
			),
		[query],
	);
	return (
		<ProductDemoFrame source="packages/ui/components/ui/graph/graph-viewer.tsx + GraphAnalyticsResult">
			<div className="overflow-hidden rounded-xl border bg-background">
				<div className="flex h-[34rem] min-h-0 w-full">
					<div className="flex min-w-0 flex-1 flex-col">
						<div className="flex flex-wrap items-center gap-2 border-b bg-background p-2">
							<label className="relative min-w-52 max-w-sm flex-1">
								<Search className="pointer-events-none absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
								<input
									value={query}
									onChange={(event) => setQuery(event.target.value)}
									placeholder="Search loaded nodes, then fallback to full graph..."
									className="h-9 w-full rounded-md border bg-transparent pl-8 pr-8 text-sm"
								/>
								{query ? (
									<button
										type="button"
										onClick={() => setQuery("")}
										className="absolute right-2 top-2.5 text-muted-foreground"
									>
										<X className="size-4" />
									</button>
								) : null}
							</label>
							{query ? (
								<span className="whitespace-nowrap text-xs text-muted-foreground">
									{matches.size} loaded matches
								</span>
							) : null}
							<span className="h-5 w-px bg-border" />
							<span className="whitespace-nowrap text-xs text-muted-foreground">
								5 nodes · 6 edges
							</span>
							<span className="h-5 w-px bg-border" />
							<select
								value={limit}
								onChange={(event) => setLimit(event.target.value)}
								className="h-8 rounded-md border bg-transparent px-2 text-xs"
							>
								<option value="50">50 nodes</option>
								<option value="100">100 nodes</option>
								<option value="200">200 nodes</option>
								<option value="500">500 nodes</option>
							</select>
							<span className="h-5 w-px bg-border" />
							<button
								type="button"
								className="whitespace-nowrap rounded border px-2 py-1 text-xs text-muted-foreground"
							>
								Query
							</button>
							<div className="ml-auto">
								<span className="whitespace-nowrap text-xs text-amber-500">
									50k edge snapshot · bounded
								</span>
							</div>
						</div>
						<div className="relative min-h-0 flex-1 overflow-hidden bg-[radial-gradient(circle_at_1px_1px,hsl(var(--border))_1px,transparent_0)] bg-[size:16px_16px]">
							<svg
								className="absolute inset-0 size-full text-border"
								aria-hidden="true"
							>
								<line
									x1="13%"
									y1="44%"
									x2="38%"
									y2="20%"
									stroke="currentColor"
								/>
								<line
									x1="38%"
									y1="20%"
									x2="63%"
									y2="52%"
									stroke="currentColor"
								/>
								<line
									x1="63%"
									y1="52%"
									x2="84%"
									y2="23%"
									stroke="currentColor"
								/>
								<line
									x1="38%"
									y1="20%"
									x2="35%"
									y2="76%"
									stroke="currentColor"
								/>
								<line
									x1="35%"
									y1="76%"
									x2="63%"
									y2="52%"
									stroke="currentColor"
								/>
								<line
									x1="13%"
									y1="44%"
									x2="35%"
									y2="76%"
									stroke="currentColor"
								/>
							</svg>
							{viewerNodes.map((node) => {
								const highlighted = !query || matches.has(node.id);
								return (
									<button
										key={node.id}
										type="button"
										onClick={() => setSelectedId(node.id)}
										aria-label={`Inspect ${node.caption}`}
										className={cn(
											"absolute flex size-16 -translate-x-1/2 -translate-y-1/2 flex-col items-center justify-center rounded-full border bg-background text-[10px] shadow-lg transition-all",
											selectedId === node.id
												? "border-primary ring-4 ring-primary/10"
												: "border-border",
											highlighted ? "opacity-100" : "opacity-25",
										)}
										style={{ left: `${node.x}%`, top: `${node.y}%` }}
									>
										<span
											className={cn("mb-1 size-2.5 rounded-full", node.color)}
										/>
										<span className="max-w-14 truncate font-semibold">
											{node.caption}
										</span>
									</button>
								);
							})}
							<div className="absolute bottom-3 left-3 z-10 rounded-md border bg-background/90 p-2 text-[10px] shadow-sm backdrop-blur">
								<p className="mb-1 font-semibold">Legend</p>
								<div className="flex gap-3">
									<span className="flex items-center gap-1">
										<span className="size-2 rounded-full bg-violet-500" /> Order
									</span>
									<span className="flex items-center gap-1">
										<span className="size-2 rounded-full bg-cyan-500" />{" "}
										Supplier
									</span>
									<span className="flex items-center gap-1">
										<span className="size-2 rounded-full bg-emerald-500" />{" "}
										Shipment
									</span>
								</div>
							</div>
						</div>
					</div>
					<aside className="hidden w-72 shrink-0 border-l bg-card sm:block">
						<div className="flex items-start justify-between border-b p-4">
							<div>
								<ProductBadge>{selected.label}</ProductBadge>
								<h3 className="mt-2 font-semibold">{selected.caption}</h3>
								<p className="font-mono text-[10px] text-muted-foreground">
									{selected.label}:{selected.id}
								</p>
							</div>
							<ProductButton
								variant="ghost"
								className="size-7 p-0"
								ariaLabel="Close inspector"
							>
								<X className="size-4" />
							</ProductButton>
						</div>
						<div className="space-y-5 p-4">
							<div>
								<p className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
									Properties
								</p>
								<div className="mt-2 space-y-2">
									<div className="flex justify-between rounded-md bg-muted/30 p-2 text-sm">
										<span className="text-muted-foreground">degree</span>
										<span className="font-mono">{selected.degree}</span>
									</div>
									<div className="flex justify-between rounded-md bg-muted/30 p-2 text-sm">
										<span className="text-muted-foreground">pagerank</span>
										<span className="font-mono">{selected.pagerank}</span>
									</div>
									<div className="flex justify-between rounded-md bg-muted/30 p-2 text-sm">
										<span className="text-muted-foreground">component</span>
										<span className="font-mono">{selected.component}</span>
									</div>
								</div>
							</div>
							<ProductButton variant="outline" className="w-full">
								<Route className="size-4" /> Find path
							</ProductButton>
						</div>
					</aside>
				</div>
				<div className="border-t bg-muted/20 p-4">
					<div className="flex flex-wrap items-center gap-2">
						<p className="mr-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
							GraphAnalyticsResult
						</p>
						<ProductBadge variant="secondary">node_count 18,402</ProductBadge>
						<ProductBadge variant="secondary">edge_count 50,000</ProductBadge>
						<ProductBadge variant="secondary">component_count 7</ProductBadge>
						<ProductBadge variant="secondary">
							isolated_node_count 312
						</ProductBadge>
						<ProductBadge className="border-amber-500/40 text-amber-600 dark:text-amber-300">
							truncated true
						</ProductBadge>
					</div>
					<p className="mt-2 text-xs text-muted-foreground">
						Exact mapped-object counts; components, degree and PageRank describe
						the explicitly bounded edge snapshot.
					</p>
				</div>
			</div>
		</ProductDemoFrame>
	);
}

function OntologySharingSurface() {
	const [exposed, setExposed] = useState(true);
	const [bindings, setBindings] = useState(true);
	const [discovered, setDiscovered] = useState(false);
	const [installed, setInstalled] = useState(false);
	return (
		<div className="grid gap-5 xl:grid-cols-[minmax(0,1.4fr)_minmax(320px,0.8fr)]">
			<div className="space-y-3">
				<div>
					<h3 className="font-semibold">Ontology contracts</h3>
					<p className="text-sm text-muted-foreground">
						Exposure controls discovery; existing connection roles still govern
						every data read and action.
					</p>
				</div>
				<ProductCard>
					<div className="space-y-4 p-4">
						<div className="flex items-start justify-between gap-3">
							<div className="flex gap-3">
								<div className="rounded-lg bg-primary/10 p-2 text-primary">
									<Share2 className="size-4" />
								</div>
								<div>
									<p className="font-medium">Order Operations</p>
									<p className="text-xs text-muted-foreground">
										4 object contracts · 2 actions
									</p>
								</div>
							</div>
							{exposed ? (
								<ProductBadge variant="secondary">Exposed</ProductBadge>
							) : (
								<ProductBadge>Private</ProductBadge>
							)}
						</div>
						<div className="h-px bg-border" />
						<div className="flex items-center justify-between gap-4">
							<span>
								<span className="block text-sm font-medium">
									Expose to connected projects
								</span>
								<span className="block text-xs text-muted-foreground">
									Allows permitted projects to discover this contract.
								</span>
							</span>
							<button
								type="button"
								role="switch"
								aria-checked={exposed}
								onClick={() => setExposed((value) => !value)}
								className={cn(
									"relative h-6 w-11 rounded-full transition-colors",
									exposed ? "bg-primary" : "bg-muted",
								)}
							>
								<span
									className={cn(
										"absolute top-1 size-4 rounded-full bg-background shadow transition-transform",
										exposed ? "translate-x-6" : "translate-x-1",
									)}
								/>
							</button>
						</div>
						<div className="flex items-center justify-between gap-4">
							<span>
								<span className="block text-sm font-medium">
									Generate board bindings
								</span>
								<span className="block text-xs text-muted-foreground">
									Adds object and action bindings to this project's node
									catalog.
								</span>
							</span>
							<button
								type="button"
								role="switch"
								aria-checked={bindings}
								onClick={() => setBindings((value) => !value)}
								className={cn(
									"relative h-6 w-11 rounded-full transition-colors",
									bindings ? "bg-primary" : "bg-muted",
								)}
							>
								<span
									className={cn(
										"absolute top-1 size-4 rounded-full bg-background shadow transition-transform",
										bindings ? "translate-x-6" : "translate-x-1",
									)}
								/>
							</button>
						</div>
					</div>
				</ProductCard>
			</div>
			<div className="space-y-4">
				<ProductCard>
					<div className="border-b p-4">
						<h3 className="flex items-center gap-2 font-semibold">
							<FileKey className="size-4" /> Connected projects
						</h3>
					</div>
					<div className="space-y-3 p-4">
						<div className="flex items-center gap-3 rounded-lg border p-3">
							<div className="rounded-lg bg-muted p-2">
								<Database className="size-4" />
							</div>
							<div className="min-w-0 flex-1">
								<p className="truncate text-sm font-medium">Invoice Intake</p>
								<p className="text-xs text-muted-foreground">
									Operations reader
								</p>
							</div>
							<CheckCircle2 className="size-4 text-emerald-500" />
						</div>
						<div className="rounded-lg bg-muted/40 p-3 text-xs text-muted-foreground">
							<p className="font-medium text-foreground">Defense in depth</p>
							<p className="mt-1">
								ReadDatabase controls object access. Exposure never widens the
								assigned role.
							</p>
						</div>
					</div>
				</ProductCard>
				{installed ? (
					<ProductCard>
						<div className="border-b p-4">
							<h3 className="flex items-center gap-2 font-semibold">
								<Layers3 className="size-4" /> Installed bindings
							</h3>
						</div>
						<div className="p-4">
							<div className="flex items-center gap-2 rounded-lg border p-3">
								<div className="min-w-0 flex-1">
									<p className="truncate text-sm font-medium">
										Order Operations
									</p>
									<p className="truncate text-xs text-muted-foreground">
										Remote · Fulfilment · 4 objects
									</p>
								</div>
								<ProductBadge variant="secondary">Installed</ProductBadge>
								<ProductButton
									variant="ghost"
									className="size-8 p-0"
									ariaLabel="Uninstall bindings"
									onClick={() => setInstalled(false)}
								>
									<Trash2 className="size-3.5" />
								</ProductButton>
							</div>
						</div>
					</ProductCard>
				) : null}
				<ProductCard>
					<div className="border-b p-4">
						<h3 className="flex items-center gap-2 font-semibold">
							<Network className="size-4" /> Available remote ontologies
						</h3>
					</div>
					<div className="space-y-3 p-4">
						<div className="rounded-lg border p-3">
							<div className="flex items-center gap-3">
								<div className="min-w-0 flex-1">
									<p className="truncate text-sm font-medium">Fulfilment</p>
									<p className="text-xs text-muted-foreground">
										Only explicitly exposed contracts are returned.
									</p>
								</div>
								<ProductButton
									variant="outline"
									className="h-8"
									onClick={() => setDiscovered(true)}
								>
									{discovered ? "Refresh" : "Discover"}
								</ProductButton>
							</div>
							{discovered ? (
								<div className="mt-3 space-y-2 border-t pt-3">
									<div className="space-y-2 rounded-md bg-muted/40 px-2.5 py-2">
										<div className="flex items-start justify-between gap-2">
											<div className="min-w-0">
												<p className="truncate text-xs font-medium">
													Order Operations
												</p>
												<p className="text-[10px] text-muted-foreground">
													4 object types · bindings only
												</p>
											</div>
											<ProductBadge
												variant={installed ? "secondary" : "outline"}
											>
												{installed ? "Installed" : "Remote"}
											</ProductBadge>
										</div>
										<div className="flex justify-end gap-2">
											<ProductButton
												variant={installed ? "outline" : "default"}
												className="h-8"
												onClick={() => setInstalled(true)}
											>
												{installed ? (
													<RefreshCw className="size-3.5" />
												) : (
													<Plus className="size-3.5" />
												)}
												{installed ? "Refresh" : "Install"}
											</ProductButton>
										</div>
									</div>
								</div>
							) : null}
						</div>
					</div>
				</ProductCard>
			</div>
		</div>
	);
}

export function RemoteOntologyDemo() {
	return (
		<ProductDemoFrame source="packages/ui/components/settings/data-studio/data-studio-panels.tsx#OntologySharingPanel">
			<div className="rounded-xl border bg-background p-4 sm:p-6">
				<OntologySharingSurface />
			</div>
		</ProductDemoFrame>
	);
}
