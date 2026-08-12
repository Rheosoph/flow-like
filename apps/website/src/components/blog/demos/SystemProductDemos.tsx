"use client";

import {
	Background,
	BackgroundVariant,
	Controls,
	type Edge,
	Handle,
	MiniMap,
	type Node,
	type NodeProps,
	Position,
	ReactFlow,
	type ReactFlowInstance,
	useNodesState,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { FlowExecutionEdge } from "@flow-like/flow-like-ui/components/flow/flow-execution-edge";
import {
	Activity,
	AppWindow,
	Archive,
	ArrowLeft,
	ArrowRight,
	ArrowUp,
	AudioLines,
	Box,
	Briefcase,
	CheckCircle2,
	Clock3,
	Code2,
	Database,
	FileClock,
	FileCode2,
	FileText,
	Files,
	FolderKanban,
	Gamepad2,
	HardDrive,
	HeartPulse,
	History,
	Home,
	Info,
	LayoutGrid,
	LayoutTemplate,
	type LucideIcon,
	MousePointer2,
	NotebookPen,
	Package,
	PanelsTopLeft,
	Paperclip,
	RefreshCw,
	Search,
	Shield,
	ShieldCheck,
	Sparkles,
	Star,
	Store,
	Trash2,
	TriangleAlert,
	Users,
	Variable,
	Waypoints,
	Wifi,
	WifiOff,
	Wrench,
	X,
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

function IconButton({
	label,
	children,
	onClick,
	className,
}: Readonly<{
	label: string;
	children: ReactNode;
	onClick?: () => void;
	className?: string;
}>) {
	return (
		<button
			type="button"
			aria-label={label}
			title={label}
			onClick={onClick}
			className={cn(
				"inline-flex size-9 items-center justify-center rounded-md border border-border bg-background text-foreground shadow-xs transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50",
				className,
			)}
		>
			{children}
		</button>
	);
}

function Button({
	children,
	onClick,
	variant = "default",
	disabled,
	className,
	type = "button",
}: Readonly<{
	children: ReactNode;
	onClick?: () => void;
	variant?: "default" | "outline" | "ghost" | "destructive" | "secondary";
	disabled?: boolean;
	className?: string;
	type?: "button" | "submit";
}>) {
	return (
		<button
			type={type}
			onClick={onClick}
			disabled={disabled}
			className={cn(
				"inline-flex min-h-9 items-center justify-center gap-2 rounded-md px-3 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50 disabled:pointer-events-none disabled:opacity-50",
				variant === "default" &&
					"bg-primary text-primary-foreground hover:bg-primary/90",
				variant === "outline" &&
					"border border-border bg-background hover:bg-muted",
				variant === "ghost" && "hover:bg-muted",
				variant === "destructive" &&
					"bg-destructive text-destructive-foreground hover:bg-destructive/90",
				variant === "secondary" &&
					"bg-secondary text-secondary-foreground hover:bg-secondary/80",
				className,
			)}
		>
			{children}
		</button>
	);
}

function Badge({
	children,
	variant = "secondary",
	className,
}: Readonly<{
	children: ReactNode;
	variant?: "secondary" | "outline";
	className?: string;
}>) {
	return (
		<span
			className={cn(
				"inline-flex min-h-5 items-center rounded-md px-2 py-0.5 text-[11px] font-medium",
				variant === "secondary" && "bg-secondary text-secondary-foreground",
				variant === "outline" && "border border-border bg-background",
				className,
			)}
		>
			{children}
		</span>
	);
}

type BlogFlowNodeData = {
	label: string;
	detail: string;
	tone: "primary" | "violet" | "tertiary";
	start?: boolean;
	hasInput?: boolean;
	hasOutput?: boolean;
};
type BlogFlowNode = Node<BlogFlowNodeData, "blogFlowNode">;
type BlogFlowEdge = Edge<Record<string, unknown>, "execution">;

const FLOW_NODE_DATA: Array<{ id: string; data: BlogFlowNodeData }> = [
	{
		id: "event",
		data: {
			label: "New order",
			detail: "event payload",
			tone: "primary",
			start: true,
			hasOutput: true,
		},
	},
	{
		id: "validate",
		data: {
			label: "Validate order",
			detail: "schema check",
			tone: "tertiary",
			hasInput: true,
			hasOutput: true,
		},
	},
	{
		id: "route",
		data: {
			label: "Route by region",
			detail: "match expression",
			tone: "violet",
			hasInput: true,
			hasOutput: true,
		},
	},
	{
		id: "inventory",
		data: {
			label: "Check inventory",
			detail: "warehouse lookup",
			tone: "tertiary",
			hasInput: true,
			hasOutput: true,
		},
	},
	{
		id: "notify",
		data: {
			label: "Notify team",
			detail: "send message",
			tone: "violet",
			hasInput: true,
			hasOutput: true,
		},
	},
	{
		id: "complete",
		data: {
			label: "Complete order",
			detail: "return result",
			tone: "primary",
			hasInput: true,
		},
	},
];

const FLOW_LAYOUTS = {
	default: {
		event: { x: 0, y: 135 },
		validate: { x: 190, y: 25 },
		route: { x: 190, y: 245 },
		inventory: { x: 380, y: 25 },
		notify: { x: 380, y: 245 },
		complete: { x: 570, y: 135 },
	},
	compact: {
		event: { x: 0, y: 115 },
		validate: { x: 170, y: 20 },
		route: { x: 170, y: 205 },
		inventory: { x: 340, y: 20 },
		notify: { x: 340, y: 205 },
		complete: { x: 510, y: 115 },
	},
} as const;

function createFlowNodes(layout: keyof typeof FLOW_LAYOUTS): BlogFlowNode[] {
	return FLOW_NODE_DATA.map((node) => ({
		id: node.id,
		type: "blogFlowNode",
		position:
			FLOW_LAYOUTS[layout][node.id as keyof typeof FLOW_LAYOUTS.default],
		data: node.data,
		zIndex: 20,
	}));
}

const FLOW_EDGES: BlogFlowEdge[] = [
	["event", "validate"],
	["event", "route"],
	["validate", "inventory"],
	["route", "notify"],
	["inventory", "complete"],
	["notify", "complete"],
].map(([source, target]) => ({
	id: `${source}-${target}`,
	source,
	target,
	sourceHandle: `${source}-out`,
	targetHandle: `${target}-in`,
	type: "execution",
	data: { pathType: "smoothstep", reduceMotion: true },
	style: {
		stroke: "var(--foreground)",
	},
	zIndex: 18,
}));

function BlogExecutionHandle({
	id,
	type,
	position,
}: Readonly<{
	id: string;
	type: "source" | "target";
	position: Position;
}>) {
	return (
		<Handle
			id={id}
			type={type}
			position={position}
			isConnectable={false}
			className="!size-3 !border-0 !bg-transparent"
		>
			<span className="pointer-events-none absolute left-1/2 top-1/2 size-2 -translate-x-1/2 -translate-y-1/2 rotate-45 rounded-[1.5px] border border-foreground bg-foreground shadow-sm" />
		</Handle>
	);
}

const BlogFlowNodeComponent = memo(
	({ id, data, selected }: NodeProps<BlogFlowNode>) => (
		<div
			className={cn(
				"relative w-40 rounded-md border border-border bg-card p-2.5 pt-8 text-left shadow-sm",
				selected && "border-2 border-primary shadow-md",
			)}
		>
			<div
				className={cn(
					"absolute inset-x-0 top-0 flex h-[18px] items-center gap-1 rounded-md rounded-b-none border-b border-border bg-card px-1.5",
					data.tone === "primary" &&
						"bg-linear-to-r from-card via-primary/50 to-primary",
					data.tone === "violet" &&
						"bg-linear-to-r from-card via-violet-500/50 to-violet-500",
					data.tone === "tertiary" &&
						"bg-linear-to-r from-card via-tertiary/50 to-tertiary",
				)}
			>
				<Waypoints className="size-3 shrink-0" />
				<span className="truncate text-[10px] font-medium leading-none">
					{data.label}
				</span>
			</div>
			{data.start ? (
				<span className="absolute -left-7 top-0 inline-flex size-5 items-center justify-center rounded-md border border-border bg-background shadow-sm">
					<ArrowRight className="size-3 text-primary" />
				</span>
			) : data.hasInput ? (
				<BlogExecutionHandle
					id={`${id}-in`}
					type="target"
					position={Position.Left}
				/>
			) : null}
			{data.hasOutput ? (
				<BlogExecutionHandle
					id={`${id}-out`}
					type="source"
					position={Position.Right}
				/>
			) : null}
			<span className="block truncate text-xs font-medium">{data.label}</span>
			<span className="mt-1 block truncate text-[10px] text-muted-foreground">
				{data.detail}
			</span>
		</div>
	),
);
BlogFlowNodeComponent.displayName = "BlogFlowNodeComponent";

const flowNodeTypes = { blogFlowNode: BlogFlowNodeComponent };
const flowEdgeTypes = { execution: FlowExecutionEdge };

function WorkflowReactFlowCanvas({ compact }: Readonly<{ compact: boolean }>) {
	const canvasRef = useRef<HTMLDivElement>(null);
	const instanceRef = useRef<ReactFlowInstance<
		BlogFlowNode,
		BlogFlowEdge
	> | null>(null);
	const [nodes, setNodes, onNodesChange] = useNodesState(
		createFlowNodes("default"),
	);

	const fit = useCallback(() => {
		instanceRef.current?.fitView({
			padding: 0.12,
			minZoom: 0.25,
			maxZoom: 1,
			duration: 180,
		});
	}, []);

	useEffect(() => {
		setNodes(createFlowNodes(compact ? "compact" : "default"));
		const timer = setTimeout(fit, 50);
		return () => clearTimeout(timer);
	}, [compact, fit, setNodes]);

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas || typeof ResizeObserver === "undefined") return;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const observer = new ResizeObserver(() => {
			if (timer) clearTimeout(timer);
			timer = setTimeout(fit, 60);
		});
		observer.observe(canvas);
		return () => {
			observer.disconnect();
			if (timer) clearTimeout(timer);
		};
	}, [fit]);

	return (
		<div ref={canvasRef} className="absolute inset-0">
			<ReactFlow<BlogFlowNode, BlogFlowEdge>
				nodes={nodes}
				edges={FLOW_EDGES}
				nodeTypes={flowNodeTypes}
				edgeTypes={flowEdgeTypes}
				onNodesChange={onNodesChange}
				onInit={(instance) => {
					instanceRef.current = instance;
					instance.fitView({ padding: 0.12, minZoom: 0.25, maxZoom: 1 });
				}}
				fitView
				fitViewOptions={{ padding: 0.12, minZoom: 0.25, maxZoom: 1 }}
				nodesDraggable
				nodesConnectable={false}
				zoomOnScroll={false}
				zoomOnPinch
				zoomOnDoubleClick={false}
				preventScrolling={false}
				deleteKeyCode={null}
				colorMode="light"
				minZoom={0.25}
				maxZoom={1.8}
				proOptions={{ hideAttribution: true }}
			>
				<Background variant={BackgroundVariant.Dots} gap={12} size={1} />
				<Controls showInteractive={false} />
				<MiniMap
					className="!hidden @3xl/board:!block"
					pannable
					zoomable
					nodeStrokeWidth={3}
				/>
			</ReactFlow>
		</div>
	);
}

function FlowDockPreview({
	onAutoLayout,
}: Readonly<{ onAutoLayout: () => void }>) {
	const items: Array<{
		label: string;
		icon: LucideIcon;
		onClick?: () => void;
		special?: boolean;
		separator?: boolean;
	}> = [
		{ label: "Variables", icon: Variable },
		{ label: "Templates", icon: LayoutTemplate },
		{ label: "Auto Layout", icon: Waypoints, onClick: onAutoLayout },
		{ label: "Manage Board", icon: NotebookPen },
		{ label: "Pages", icon: FileText },
		{ label: "Search", icon: Search },
		{ label: "FlowScript", icon: FileCode2 },
		{ label: "Run History", icon: History, separator: true },
		{ label: "FlowPilot", icon: Sparkles, separator: true, special: true },
	];

	return (
		<div className="relative z-30 flex min-h-14 w-full shrink-0 items-center justify-start gap-1 overflow-x-auto border-b border-border bg-card px-3 py-2 shadow-sm [scrollbar-width:none] [&::-webkit-scrollbar]:hidden @lg/board:justify-center @5xl/board:gap-2">
			{items.map((item) => (
				<span key={item.label} className="flex shrink-0 items-center gap-2">
					{item.separator ? (
						<span className="h-8 w-px rounded-full bg-border" />
					) : null}
					<button
						type="button"
						title={item.label}
						aria-label={item.label}
						onClick={item.onClick}
						className={cn(
							"flex size-8 items-center justify-center rounded-full bg-secondary text-secondary-foreground transition hover:-translate-y-1 hover:scale-110",
							item.special &&
								"bg-linear-to-br from-primary via-violet-500 to-pink-500 text-white",
						)}
					>
						<item.icon className="size-4" />
					</button>
				</span>
			))}
		</div>
	);
}

type BoardNotice = "warning" | "offline" | "success" | null;

function BoardToast({
	state,
	onRetry,
}: Readonly<{ state: BoardNotice; onRetry: () => void }>) {
	if (!state) return null;
	const content =
		state === "warning"
			? "Server sync failed — your edits are queued and will retry on the next board load."
			: state === "offline"
				? "Cannot sync queued edits while offline or signed out."
				: "Queued edits synced to the server.";

	return (
		<output
			aria-live="polite"
			className="relative z-40 grid shrink-0 grid-cols-[auto_minmax(0,1fr)] items-start gap-3 border-t border-border bg-popover p-4 text-popover-foreground shadow-[0_-8px_24px_-20px_rgba(0,0,0,0.45)] @md/board:grid-cols-[auto_minmax(0,1fr)_auto]"
		>
			{state === "success" ? (
				<CheckCircle2 className="mt-0.5 size-4 shrink-0 text-emerald-500" />
			) : (
				<TriangleAlert className="mt-0.5 size-4 shrink-0 text-amber-500" />
			)}
			<p className="min-w-0 flex-1 text-xs font-medium leading-relaxed sm:text-sm">
				{content}
			</p>
			{state === "warning" ? (
				<Button
					variant="outline"
					className="col-start-2 min-h-8 shrink-0 justify-self-start px-2.5 text-xs @md/board:col-start-3 @md/board:row-start-1"
					onClick={onRetry}
				>
					Retry now
				</Button>
			) : null}
		</output>
	);
}

function BoardStatus({
	connection,
	onReconnect,
	showActivity,
}: Readonly<{
	connection: "live" | "disconnected";
	onReconnect?: () => void;
	showActivity?: boolean;
}>) {
	return (
		<div className="relative z-30 flex min-h-11 shrink-0 flex-wrap items-center justify-end gap-2 border-b border-border bg-background/95 px-3 py-2">
			{connection === "live" ? (
				<div className="flex items-center gap-2 rounded-xl border border-primary/35 bg-background/90 px-3 py-1.5 shadow-sm">
					<Wifi className="size-3.5 animate-pulse text-primary" />
					<span className="text-xs font-medium text-primary">Live</span>
				</div>
			) : (
				<button
					type="button"
					onClick={onReconnect}
					className="flex items-center gap-2 rounded-xl border border-destructive/35 bg-background/90 px-3 py-1.5 shadow-sm transition-colors hover:bg-background/80"
				>
					<WifiOff className="size-3.5 text-destructive" />
					<span className="text-xs font-medium text-destructive">
						Disconnected
						<span className="hidden @md/board:inline">
							{" "}
							- Click to reconnect
						</span>
					</span>
				</button>
			)}
			{showActivity ? (
				<div className="flex items-center gap-2 rounded-xl border border-green-500/35 bg-green-500/10 px-3 py-1.5 shadow-sm backdrop-blur-sm">
					<Activity className="size-3.5 animate-pulse text-green-500" />
					<span className="text-xs font-medium text-green-600 dark:text-green-400">
						1 run • 2 active • 18 exec
					</span>
				</div>
			) : null}
		</div>
	);
}

function FlowBoardSurface({
	connection,
	onReconnect,
	notice,
	onRetry,
	showPresence = false,
	showActivity = false,
}: Readonly<{
	connection: "live" | "disconnected";
	onReconnect?: () => void;
	notice?: BoardNotice;
	onRetry?: () => void;
	showPresence?: boolean;
	showActivity?: boolean;
}>) {
	const [compactLayout, setCompactLayout] = useState(false);

	return (
		<div className="@container/board">
			<div
				className={cn(
					"relative flex overflow-hidden rounded-2xl border border-border bg-background shadow-sm",
					notice
						? "h-[46rem] flex-col @xl/board:h-[39rem]"
						: "h-[40rem] flex-col @xl/board:h-[34rem]",
				)}
			>
				<FlowDockPreview
					onAutoLayout={() => setCompactLayout((value) => !value)}
				/>
				<BoardStatus
					connection={connection}
					onReconnect={onReconnect}
					showActivity={showActivity}
				/>

				<div className="relative min-h-0 flex-1">
					<WorkflowReactFlowCanvas compact={compactLayout} />
					{showPresence ? (
						<>
							<div className="pointer-events-none absolute left-[57%] top-[36%] z-20 text-sky-500">
								<MousePointer2 className="size-5 fill-current" />
								<span className="ml-3 -mt-1 block rounded-full bg-sky-500 px-2 py-0.5 text-[9px] font-semibold text-white shadow-sm">
									Maya
								</span>
							</div>
							<div className="absolute right-4 top-4 z-20 flex -space-x-2">
								<span className="flex size-7 items-center justify-center rounded-full border-2 border-background bg-violet-500 text-[9px] font-bold text-white">
									MA
								</span>
								<span className="flex size-7 items-center justify-center rounded-full border-2 border-background bg-cyan-500 text-[9px] font-bold text-white">
									FL
								</span>
							</div>
						</>
					) : null}
				</div>

				{notice && onRetry ? (
					<BoardToast state={notice} onRetry={onRetry} />
				) : null}
			</div>
		</div>
	);
}

export function BoardSyncDemo() {
	const [connected, setConnected] = useState(false);
	const [notice, setNotice] = useState<BoardNotice>("warning");

	return (
		<ProductDemoFrame source="FlowBoard · desktop board sync status">
			<FlowBoardSurface
				connection={connected ? "live" : "disconnected"}
				onReconnect={() => setConnected(true)}
				notice={notice}
				onRetry={() => setNotice(connected ? "success" : "offline")}
			/>
		</ProductDemoFrame>
	);
}

export function RenderPerformanceDemo() {
	return (
		<ProductDemoFrame source="FlowCanvas · FlowNode · FlowDock">
			<FlowBoardSurface connection="live" showPresence showActivity />
		</ProductDemoFrame>
	);
}

type StorageCategoryKey =
	| "apps"
	| "bits"
	| "logs"
	| "offloaded"
	| "browser"
	| "cache"
	| "temporary";

type StorageItem = {
	id: string;
	name: string;
	detail: string;
	sizeBytes: number;
	updated: string;
	deletable: boolean;
};

type StorageCategory = {
	key: StorageCategoryKey;
	label: string;
	description: string;
	sizeBytes: number;
	icon: LucideIcon;
	color: string;
	soft: string;
	items: StorageItem[];
};

const GIB = 1024 ** 3;
const MIB = 1024 ** 2;

const STORAGE_CATEGORIES: StorageCategory[] = [
	{
		key: "apps",
		label: "Apps & projects",
		description: "Local boards, databases, media, and app-owned files.",
		sizeBytes: 5.8 * GIB,
		icon: FolderKanban,
		color: "bg-sky-500",
		soft: "bg-sky-500/10 text-sky-600 dark:text-sky-400",
		items: [
			{
				id: "app:ops",
				name: "Order operations",
				detail: "12 boards · 3 databases · local project",
				sizeBytes: 3.1 * GIB,
				updated: "Today, 09:42",
				deletable: true,
			},
			{
				id: "app:media",
				name: "Media workshop",
				detail: "8 boards · 42 media files",
				sizeBytes: 2.7 * GIB,
				updated: "Yesterday, 18:20",
				deletable: true,
			},
		],
	},
	{
		key: "bits",
		label: "Downloaded bits",
		description: "Model weights and reusable runtime artifacts.",
		sizeBytes: 4.2 * GIB,
		icon: Box,
		color: "bg-violet-500",
		soft: "bg-violet-500/10 text-violet-600 dark:text-violet-400",
		items: [
			{
				id: "bit:vision",
				name: "Local vision model",
				detail: "GGUF model · downloaded bit",
				sizeBytes: 3.4 * GIB,
				updated: "Jul 18, 16:08",
				deletable: true,
			},
			{
				id: "bit:speech",
				name: "Speech runtime",
				detail: "Whisper runtime · downloaded bit",
				sizeBytes: 0.8 * GIB,
				updated: "Jul 17, 12:14",
				deletable: true,
			},
		],
	},
	{
		key: "logs",
		label: "Run logs",
		description: "Per-run local execution and debug history.",
		sizeBytes: 1.9 * GIB,
		icon: FileClock,
		color: "bg-amber-500",
		soft: "bg-amber-500/10 text-amber-600 dark:text-amber-400",
		items: [
			{
				id: "run:20260720",
				name: "run_01K0M8F1",
				detail: "Order operations · completed run",
				sizeBytes: 822 * MIB,
				updated: "Today, 10:04",
				deletable: true,
			},
			{
				id: "run:active",
				name: "run_01K0M7XE",
				detail: "Media workshop · currently active",
				sizeBytes: 614 * MIB,
				updated: "Today, 10:11",
				deletable: false,
			},
			{
				id: "run:older",
				name: "run_01JZYK2Q",
				detail: "Customer briefing · completed run",
				sizeBytes: 510 * MIB,
				updated: "Jun 14, 08:32",
				deletable: true,
			},
		],
	},
	{
		key: "offloaded",
		label: "Offloaded browser files",
		description: "Large IndexedDB payloads moved to native disk.",
		sizeBytes: 620 * MIB,
		icon: Files,
		color: "bg-fuchsia-500",
		soft: "bg-fuchsia-500/10 text-fuchsia-600 dark:text-fuchsia-400",
		items: [
			{
				id: "offload:payloads",
				name: "IndexedDB payload offload",
				detail: "Managed by Studio",
				sizeBytes: 620 * MIB,
				updated: "Today, 09:58",
				deletable: false,
			},
		],
	},
	{
		key: "browser",
		label: "Browser storage",
		description: "Local preferences and IndexedDB records in this WebView.",
		sizeBytes: 430 * MIB,
		icon: PanelsTopLeft,
		color: "bg-cyan-500",
		soft: "bg-cyan-500/10 text-cyan-600 dark:text-cyan-400",
		items: [
			{
				id: "browser:flow-like",
				name: "flow-like",
				detail: "IndexedDB · 8 stores · 1,248 records",
				sizeBytes: 430 * MIB,
				updated: "Today, 10:10",
				deletable: false,
			},
		],
	},
	{
		key: "cache",
		label: "Cache & support data",
		description: "Rebuildable caches and supporting local databases.",
		sizeBytes: 780 * MIB,
		icon: Database,
		color: "bg-emerald-500",
		soft: "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
		items: [
			{
				id: "cache:catalog",
				name: "Catalog cache",
				detail: "Managed supporting database",
				sizeBytes: 780 * MIB,
				updated: "Today, 09:57",
				deletable: false,
			},
		],
	},
	{
		key: "temporary",
		label: "Temporary files",
		description: "Intermediate files created while workflows run.",
		sizeBytes: 122 * MIB,
		icon: Archive,
		color: "bg-rose-500",
		soft: "bg-rose-500/10 text-rose-600 dark:text-rose-400",
		items: [
			{
				id: "tmp:render",
				name: "video-render-01K0M",
				detail: "Completed workflow intermediate",
				sizeBytes: 122 * MIB,
				updated: "Today, 08:21",
				deletable: true,
			},
		],
	},
];

function humanFileSize(bytes: number) {
	if (bytes >= GIB) return `${(bytes / GIB).toFixed(1)} GB`;
	if (bytes >= MIB) return `${Math.round(bytes / MIB)} MB`;
	return `${Math.round(bytes / 1024)} KB`;
}

function CheckBox({
	checked,
	disabled,
	label,
	onChange,
}: Readonly<{
	checked: boolean;
	disabled?: boolean;
	label: string;
	onChange: (checked: boolean) => void;
}>) {
	return (
		<input
			type="checkbox"
			checked={checked}
			disabled={disabled}
			aria-label={label}
			onChange={(event) => onChange(event.target.checked)}
			className="size-4 rounded border-border accent-primary disabled:opacity-40"
		/>
	);
}

export function StorageOverviewDemo() {
	const [activeCategory, setActiveCategory] =
		useState<StorageCategoryKey>("logs");
	const [selected, setSelected] = useState<Set<string>>(new Set());
	const [deleted, setDeleted] = useState<Set<string>>(new Set());
	const [search, setSearch] = useState("");
	const [pendingDelete, setPendingDelete] = useState<StorageItem[] | null>(
		null,
	);

	const categories = useMemo(
		() =>
			STORAGE_CATEGORIES.map((category) => {
				const removedBytes = category.items
					.filter((item) => deleted.has(item.id))
					.reduce((total, item) => total + item.sizeBytes, 0);
				return {
					...category,
					sizeBytes: Math.max(0, category.sizeBytes - removedBytes),
					items: category.items.filter((item) => !deleted.has(item.id)),
				};
			}),
		[deleted],
	);
	const category =
		categories.find((item) => item.key === activeCategory) ?? categories[0];
	const filteredItems = (category?.items ?? []).filter((item) => {
		const query = search.trim().toLocaleLowerCase();
		return (
			!query ||
			item.name.toLocaleLowerCase().includes(query) ||
			item.detail.toLocaleLowerCase().includes(query)
		);
	});
	const selectedItems = (category?.items ?? []).filter((item) =>
		selected.has(item.id),
	);
	const selectedBytes = selectedItems.reduce(
		(total, item) => total + item.sizeBytes,
		0,
	);
	const totalBytes = categories.reduce(
		(total, item) => total + item.sizeBytes,
		0,
	);

	const changeCategory = (key: StorageCategoryKey) => {
		setActiveCategory(key);
		setSelected(new Set());
		setSearch("");
	};

	return (
		<ProductDemoFrame source="Settings → Local storage">
			<div className="relative overflow-hidden rounded-2xl border border-border bg-background p-4 shadow-sm sm:p-5">
				<div className="flex items-start justify-between gap-4 pb-5">
					<div>
						<button
							type="button"
							className="mb-2 inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground"
						>
							<ArrowLeft className="size-4" /> Settings
						</button>
						<h3 className="text-2xl font-bold tracking-tight sm:text-3xl">
							Local storage
						</h3>
						<p className="mt-1 text-sm text-muted-foreground">
							Understand what Studio keeps on this device and take control of
							it.
						</p>
					</div>
					<Button
						variant="outline"
						className="shrink-0"
						onClick={() => {
							setSearch("");
							setSelected(new Set());
						}}
					>
						<RefreshCw className="size-4" /> Refresh
					</Button>
				</div>

				<section className="relative overflow-hidden rounded-2xl border border-border bg-gradient-to-br from-card via-card to-primary/[0.07] p-5 shadow-sm md:p-7">
					<div className="pointer-events-none absolute -right-24 -top-28 size-80 rounded-full bg-primary/10 blur-3xl" />
					<div className="relative grid items-center gap-8 lg:grid-cols-[minmax(0,1fr)_auto]">
						<div className="space-y-5">
							<div className="flex items-start gap-3">
								<div className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-primary text-primary-foreground shadow-sm">
									<HardDrive className="size-5" />
								</div>
								<div>
									<p className="text-sm font-medium text-muted-foreground">
										Used by Flow-Like Studio
									</p>
									<p className="text-4xl font-semibold tracking-tight md:text-5xl">
										{humanFileSize(totalBytes)}
									</p>
								</div>
							</div>
							<div className="grid gap-x-7 gap-y-3 sm:grid-cols-2 lg:grid-cols-3">
								{categories.map((entry) => (
									<button
										type="button"
										key={`summary-${entry.key}`}
										onClick={() => changeCategory(entry.key)}
										className="group flex min-w-0 items-center gap-2 text-left"
									>
										<span
											className={cn("size-2.5 rounded-full", entry.color)}
										/>
										<span className="truncate text-sm text-muted-foreground group-hover:text-foreground">
											{entry.label}
										</span>
										<span className="ml-auto text-sm font-semibold tabular-nums">
											{humanFileSize(entry.sizeBytes)}
										</span>
									</button>
								))}
							</div>
						</div>
						<div className="hidden size-36 rounded-full border-[14px] border-primary/10 p-3 lg:flex lg:items-center lg:justify-center">
							<div className="flex size-full flex-col items-center justify-center rounded-full bg-background/80 text-center shadow-inner">
								<span className="text-lg font-semibold">
									{categories.reduce((sum, item) => sum + item.items.length, 0)}
								</span>
								<span className="text-xs text-muted-foreground">
									stored items
								</span>
							</div>
						</div>
					</div>
				</section>

				<div className="mt-5 grid gap-3 sm:grid-cols-2 lg:grid-cols-4 xl:grid-cols-7">
					{categories.map((entry) => (
						<button
							type="button"
							key={entry.key}
							onClick={() => changeCategory(entry.key)}
							className={cn(
								"rounded-xl border border-border bg-card p-4 text-left shadow-xs transition-all hover:-translate-y-0.5 hover:shadow-md",
								activeCategory === entry.key &&
									"border-primary ring-2 ring-primary/15",
							)}
						>
							<span
								className={cn(
									"mb-4 flex size-9 items-center justify-center rounded-lg",
									entry.soft,
								)}
							>
								<entry.icon className="size-[18px]" />
							</span>
							<span className="block truncate text-sm font-medium">
								{entry.label}
							</span>
							<span className="mt-1 flex items-baseline justify-between gap-2">
								<span className="text-lg font-semibold tabular-nums">
									{humanFileSize(entry.sizeBytes)}
								</span>
								<span className="text-xs text-muted-foreground">
									{entry.items.length} items
								</span>
							</span>
						</button>
					))}
				</div>

				<div className="mt-5 overflow-hidden rounded-xl border border-border bg-card">
					<div className="border-b border-border bg-muted/20 p-4">
						<div className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
							<div>
								<div className="flex items-center gap-2">
									<h4 className="font-semibold">{category?.label}</h4>
									<Badge>{category?.items.length ?? 0}</Badge>
								</div>
								<p className="mt-1 max-w-2xl text-sm text-muted-foreground">
									{category?.description}
								</p>
							</div>
							{selectedItems.length > 0 ? (
								<Button
									variant="destructive"
									onClick={() => setPendingDelete(selectedItems)}
								>
									<Trash2 className="size-4" /> Delete {selectedItems.length} ·{" "}
									{humanFileSize(selectedBytes)}
								</Button>
							) : null}
						</div>
						<label className="relative mt-4 block">
							<span className="sr-only">Search {category?.label}</span>
							<Search className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
							<input
								value={search}
								onChange={(event) => setSearch(event.target.value)}
								placeholder={`Search ${category?.label.toLocaleLowerCase() ?? "files"}`}
								className="h-10 w-full rounded-md border border-input bg-background pl-9 pr-3 text-sm outline-none focus:ring-2 focus:ring-primary/40"
							/>
						</label>
					</div>

					{filteredItems.length === 0 ? (
						<div className="flex min-h-56 flex-col items-center justify-center px-6 text-center">
							<div className="mb-3 flex size-11 items-center justify-center rounded-full bg-muted text-muted-foreground">
								<CheckCircle2 className="size-5" />
							</div>
							<p className="font-medium">
								{search ? "No matching items" : "Nothing stored here"}
							</p>
							<p className="mt-1 text-sm text-muted-foreground">
								{search
									? "Try a different search."
									: "This category is already clean."}
							</p>
						</div>
					) : (
						<div className="divide-y divide-border">
							<div className="grid grid-cols-[32px_minmax(0,1fr)_88px_40px] items-center gap-3 px-4 py-2.5 text-xs font-medium uppercase tracking-wide text-muted-foreground sm:grid-cols-[32px_minmax(0,1fr)_100px_138px_40px]">
								<CheckBox
									label="Select all visible items"
									checked={
										filteredItems.filter((item) => item.deletable).length > 0 &&
										filteredItems
											.filter((item) => item.deletable)
											.every((item) => selected.has(item.id))
									}
									onChange={(checked) => {
										const next = new Set(selected);
										for (const item of filteredItems.filter(
											(entry) => entry.deletable,
										)) {
											if (checked) next.add(item.id);
											else next.delete(item.id);
										}
										setSelected(next);
									}}
								/>
								<span>Item</span>
								<span className="text-right">Size</span>
								<span className="hidden text-right sm:block">Last changed</span>
								<span />
							</div>
							{filteredItems.map((item) => (
								<div
									key={item.id}
									className="grid grid-cols-[32px_minmax(0,1fr)_88px_40px] items-center gap-3 px-4 py-3.5 transition-colors hover:bg-muted/30 sm:grid-cols-[32px_minmax(0,1fr)_100px_138px_40px]"
								>
									<CheckBox
										label={`Select ${item.name}`}
										checked={selected.has(item.id)}
										disabled={!item.deletable}
										onChange={(checked) => {
											const next = new Set(selected);
											if (checked) next.add(item.id);
											else next.delete(item.id);
											setSelected(next);
										}}
									/>
									<div className="min-w-0">
										<div className="flex items-center gap-2">
											<p className="truncate text-sm font-medium">
												{item.name}
											</p>
											{!item.deletable ? (
												<Badge variant="outline">
													{activeCategory === "logs" ? "In use" : "Managed"}
												</Badge>
											) : null}
										</div>
										<p className="mt-0.5 truncate text-xs text-muted-foreground">
											{item.detail}
										</p>
									</div>
									<p className="text-right text-sm font-medium tabular-nums">
										{humanFileSize(item.sizeBytes)}
									</p>
									<p className="hidden text-right text-xs text-muted-foreground sm:block">
										{item.updated}
									</p>
									<IconButton
										label={`Delete ${item.name}`}
										onClick={
											item.deletable
												? () => setPendingDelete([item])
												: undefined
										}
										className={cn(
											"size-8 border-0 shadow-none",
											!item.deletable && "pointer-events-none opacity-35",
										)}
									>
										<Trash2 className="size-4" />
									</IconButton>
								</div>
							))}
						</div>
					)}
				</div>

				{pendingDelete ? (
					<div className="absolute inset-0 z-50 flex items-center justify-center bg-background/70 p-4 backdrop-blur-sm">
						<div
							role="alertdialog"
							aria-modal="true"
							aria-labelledby="storage-delete-title"
							className="w-full max-w-lg rounded-xl border border-border bg-background p-6 shadow-2xl"
						>
							<h4 id="storage-delete-title" className="text-lg font-semibold">
								Delete {pendingDelete.length} local{" "}
								{pendingDelete.length === 1 ? "item" : "items"}?
							</h4>
							<p className="mt-2 text-sm leading-relaxed text-muted-foreground">
								This will permanently remove{" "}
								{humanFileSize(
									pendingDelete.reduce((sum, item) => sum + item.sizeBytes, 0),
								)}{" "}
								from this device.
							</p>
							<div className="mt-6 flex justify-end gap-2">
								<Button
									variant="outline"
									onClick={() => setPendingDelete(null)}
								>
									Cancel
								</Button>
								<Button
									variant="destructive"
									onClick={() => {
										setDeleted(
											(current) =>
												new Set([
													...current,
													...pendingDelete.map((item) => item.id),
												]),
										);
										setSelected(new Set());
										setPendingDelete(null);
									}}
								>
									Delete permanently
								</Button>
							</div>
						</div>
					</div>
				) : null}
			</div>
		</ProductDemoFrame>
	);
}

function Switch({
	checked,
	onChange,
	disabled,
	label,
}: Readonly<{
	checked: boolean;
	onChange: (checked: boolean) => void;
	disabled?: boolean;
	label: string;
}>) {
	return (
		<button
			type="button"
			role="switch"
			aria-checked={checked}
			aria-label={label}
			disabled={disabled}
			onClick={() => onChange(!checked)}
			className={cn(
				"relative h-6 w-11 shrink-0 rounded-full border-2 border-transparent transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50 disabled:opacity-50",
				checked ? "bg-primary" : "bg-input",
			)}
		>
			<span
				className={cn(
					"pointer-events-none block size-5 rounded-full bg-background shadow-lg transition-transform",
					checked ? "translate-x-5" : "translate-x-0",
				)}
			/>
		</button>
	);
}

export function StorageRetentionDemo() {
	const [enabled, setEnabled] = useState(false);
	const [days, setDays] = useState(30);
	const [savedDays, setSavedDays] = useState(30);
	const [lastChecked, setLastChecked] = useState("Jul 20, 2026, 10:12");

	const save = (nextDays = days) => {
		setDays(nextDays);
		setSavedDays(nextDays);
		setLastChecked("Just now");
	};

	return (
		<ProductDemoFrame source="Settings → Local storage → Automatic log cleanup">
			<div className="rounded-2xl border border-border bg-background p-5 shadow-sm">
				<div className="mx-auto grid max-w-3xl gap-5 md:grid-cols-[minmax(0,1fr)_360px] md:items-start">
					<div>
						<button
							type="button"
							className="mb-2 inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground"
						>
							<ArrowLeft className="size-4" /> Settings
						</button>
						<h3 className="text-3xl font-bold tracking-tight">Local storage</h3>
						<p className="mt-1 text-sm text-muted-foreground">
							Understand what Studio keeps on this device and take control of
							it.
						</p>
						<div className="mt-6 rounded-xl border border-dashed border-border bg-muted/20 p-4">
							<div className="flex gap-3">
								<Info className="mt-0.5 size-4 shrink-0 text-primary" />
								<div className="space-y-1">
									<p className="text-sm font-medium">
										Everything shown here is local
									</p>
									<p className="text-xs leading-relaxed text-muted-foreground">
										This page inspects files on this device only. It does not
										count or delete cloud data.
									</p>
								</div>
							</div>
						</div>
					</div>

					<div className="overflow-hidden rounded-xl border border-amber-500/25 bg-card">
						<div className="h-1 bg-gradient-to-r from-amber-400 via-orange-400 to-rose-400" />
						<div className="p-6 pb-4">
							<div className="flex items-start justify-between gap-4">
								<div className="flex gap-3">
									<div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-amber-500/10 text-amber-600 dark:text-amber-400">
										<Sparkles className="size-[18px]" />
									</div>
									<div>
										<h4 className="text-base font-semibold">
											Automatic log cleanup
										</h4>
										<p className="mt-1 text-sm text-muted-foreground">
											Keep debug runs from growing unnoticed.
										</p>
									</div>
								</div>
								<Switch
									checked={enabled}
									onChange={(checked) => {
										setEnabled(checked);
										setLastChecked("Just now");
									}}
									label="Automatically delete old run logs"
								/>
							</div>
						</div>
						<div className="space-y-4 p-6 pt-2">
							<div
								className={cn(
									"space-y-2 transition-opacity",
									!enabled && "opacity-50",
								)}
							>
								<label
									htmlFor="blog-retention-days"
									className="text-sm font-medium"
								>
									Delete run logs older than
								</label>
								<div className="grid grid-cols-4 gap-2">
									{[7, 14, 30, 60].map((option) => (
										<Button
											key={option}
											variant={days === option ? "default" : "outline"}
											className="min-h-8 px-2 text-xs"
											disabled={!enabled}
											onClick={() => save(option)}
										>
											{option}d
										</Button>
									))}
								</div>
								<div className="flex items-center gap-2">
									<input
										id="blog-retention-days"
										type="number"
										min={1}
										max={3650}
										value={days}
										disabled={!enabled}
										onChange={(event) =>
											setDays(
												Math.max(
													1,
													Math.min(
														3650,
														Number.parseInt(event.target.value || "1", 10),
													),
												),
											)
										}
										className="h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm outline-none focus:ring-2 focus:ring-primary/40 disabled:cursor-not-allowed disabled:opacity-50"
									/>
									<Button
										variant="secondary"
										disabled={!enabled || days === savedDays}
										onClick={() => save()}
									>
										Save
									</Button>
								</div>
							</div>
							<div className="flex gap-2.5 rounded-lg bg-muted/60 p-3 text-xs text-muted-foreground">
								<ShieldCheck className="mt-0.5 size-4 shrink-0 text-foreground" />
								<p>
									Only completed local run logs are removed. Apps, boards, and
									active runs are never part of automatic cleanup.
								</p>
							</div>
							<p className="flex items-center gap-1.5 text-xs text-muted-foreground">
								<Clock3 className="size-3.5" /> Last checked {lastChecked}
							</p>
						</div>
					</div>
				</div>
			</div>
		</ProductDemoFrame>
	);
}

type ExploreSurface = "home" | "explore";
type ExploreTab = "apps" | "packages";

const CATEGORY_FIXTURES: Array<{
	label: string;
	icon: LucideIcon;
	color: string;
}> = [
	{ label: "Productivity", icon: Sparkles, color: "oklch(0.65 0.15 240)" },
	{ label: "Business", icon: Briefcase, color: "oklch(0.65 0.15 250)" },
	{ label: "Utilities", icon: Wrench, color: "oklch(0.65 0.10 230)" },
	{ label: "Communication", icon: Users, color: "oklch(0.65 0.15 290)" },
	{ label: "Games", icon: Gamepad2, color: "oklch(0.65 0.15 310)" },
	{ label: "Health", icon: HeartPulse, color: "oklch(0.65 0.15 145)" },
];

type AppFixture = {
	id: string;
	name: string;
	description: string;
	category: string;
	rating: string;
	reviews: string;
	color: string;
	accent: string;
};

const APP_FIXTURES: AppFixture[] = [
	{
		id: "research",
		name: "Research Copilot",
		description: "Search, compare, and turn evidence into a clear briefing.",
		category: "Productivity",
		rating: "4.9",
		reviews: "128",
		color: "from-violet-600 via-indigo-500 to-cyan-400",
		accent: "#a78bfa",
	},
	{
		id: "revenue",
		name: "Revenue Studio",
		description: "Live operating metrics, forecasts, and decision workflows.",
		category: "Business",
		rating: "4.8",
		reviews: "84",
		color: "from-sky-600 via-cyan-500 to-emerald-400",
		accent: "#22d3ee",
	},
	{
		id: "pipeline",
		name: "Content Pipeline",
		description: "Take a campaign from brief to approval and publish.",
		category: "Productivity",
		rating: "4.7",
		reviews: "61",
		color: "from-rose-500 via-orange-400 to-amber-300",
		accent: "#fb7185",
	},
	{
		id: "support",
		name: "Support Console",
		description: "Triage conversations and coordinate customer follow-up.",
		category: "Communication",
		rating: "4.9",
		reviews: "96",
		color: "from-emerald-600 via-teal-500 to-sky-400",
		accent: "#34d399",
	},
];

type PackageFixture = {
	name: string;
	category: string;
	description: string;
	version: string;
	installs: string;
	rating: string;
	color: string;
};

const PACKAGE_FIXTURES: PackageFixture[] = [
	{
		name: "flow-like/data-toolkit",
		category: "Data",
		description:
			"Typed transformation, table, and ontology nodes for production workflows.",
		version: "0.8.2",
		installs: "12.4k",
		rating: "4.9",
		color: "from-cyan-600 via-blue-600 to-violet-600",
	},
	{
		name: "community/media-lab",
		category: "Media",
		description:
			"Composable audio, image, and video operations compiled to WASM.",
		version: "1.3.0",
		installs: "8.7k",
		rating: "4.8",
		color: "from-fuchsia-600 via-rose-500 to-amber-400",
	},
];

function MiniAppCard({
	app,
	compact = false,
}: Readonly<{ app: AppFixture; compact?: boolean }>) {
	if (compact) {
		return (
			<button
				type="button"
				className="group relative flex min-w-0 flex-1 items-center gap-3 overflow-hidden rounded-xl border border-border/40 bg-card/80 p-3 text-left backdrop-blur-sm transition-all hover:border-primary/20 hover:bg-card/95 hover:shadow-md"
			>
				<span
					className={cn(
						"absolute inset-y-0 left-0 w-24 bg-gradient-to-r opacity-25",
						app.color,
					)}
				/>
				<span
					className={cn(
						"relative flex size-11 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br text-xs font-bold text-white shadow-sm",
						app.color,
					)}
				>
					{app.name.slice(0, 2).toUpperCase()}
				</span>
				<span className="relative min-w-0 flex-1">
					<span className="block truncate text-sm font-semibold">
						{app.name}
					</span>
					<span className="mt-0.5 block truncate text-xs text-muted-foreground">
						{app.description}
					</span>
				</span>
				<span className="relative shrink-0 rounded-full bg-muted/30 px-2.5 py-0.5 text-xs font-medium text-muted-foreground">
					GET
				</span>
			</button>
		);
	}

	return (
		<button
			type="button"
			className="group relative flex min-h-72 w-56 shrink-0 snap-start flex-col justify-end overflow-hidden rounded-xl border border-border/40 bg-card text-left shadow-sm transition-all duration-300 hover:-translate-y-1 hover:border-primary/30 hover:shadow-xl sm:w-64"
		>
			<span
				className={cn(
					"absolute inset-0 bg-gradient-to-br transition-transform duration-500 group-hover:scale-105",
					app.color,
				)}
			/>
			<span className="absolute inset-0 bg-gradient-to-t from-black/90 via-black/40 to-black/5" />
			<span className="relative z-10 flex flex-col gap-2.5 p-5">
				<span className="flex items-center gap-3">
					<span className="flex size-11 shrink-0 items-center justify-center rounded-xl border border-white/15 bg-white/15 text-sm font-bold text-white shadow-lg backdrop-blur-md">
						{app.name.slice(0, 2).toUpperCase()}
					</span>
					<span className="min-w-0">
						<span
							className="block truncate text-[11px] font-semibold uppercase tracking-wider"
							style={{ color: app.accent }}
						>
							{app.category}
						</span>
						<span className="block truncate text-lg font-bold leading-tight text-white">
							{app.name}
						</span>
					</span>
				</span>
				<span className="line-clamp-2 min-h-10 text-sm leading-relaxed text-white/75">
					{app.description}
				</span>
				<span className="mt-1 flex items-center justify-between">
					<span className="flex items-center gap-1.5 text-sm text-white/90">
						<Star className="size-4 fill-yellow-400 text-yellow-400" />
						<strong>{app.rating}</strong>
						<span className="text-xs text-white/50">({app.reviews})</span>
					</span>
					<span className="rounded-full border border-white/25 bg-white/15 px-3 py-1 text-xs font-semibold text-white backdrop-blur-sm">
						GET
					</span>
				</span>
			</span>
		</button>
	);
}

function SectionHeader({
	title,
	subtitle,
	action,
}: Readonly<{ title: string; subtitle?: string; action?: string }>) {
	return (
		<div className="mb-4 flex items-end justify-between gap-4">
			<div className="min-w-0 space-y-1">
				<h4 className="text-xl font-bold tracking-tight">{title}</h4>
				{subtitle ? (
					<p className="text-sm text-muted-foreground">{subtitle}</p>
				) : null}
			</div>
			{action ? (
				<button
					type="button"
					className="group flex shrink-0 items-center gap-1.5 rounded-full border border-border/40 bg-card/60 px-4 py-1.5 text-sm font-medium text-muted-foreground hover:border-primary/30 hover:text-foreground hover:shadow-sm"
				>
					{action}
					<ArrowRight className="size-3.5 transition-transform group-hover:translate-x-0.5" />
				</button>
			) : null}
		</div>
	);
}

function FlowPilotHero() {
	const [open, setOpen] = useState(false);
	const [prompt, setPrompt] = useState("");
	const suggestions = [
		"Create a new app",
		"What can I build?",
		"Show me the package store",
		"Switch my profile",
	];

	return (
		<div className="flex w-full shrink-0 flex-col items-center gap-5 overflow-x-clip px-4 pb-8 pt-10">
			<div className="flex flex-col items-center gap-2 text-center">
				<h3 className="text-[26px] font-bold tracking-tight text-balance sm:text-3xl">
					What do you want to <span className="text-primary">build</span>?
				</h3>
				<p className="text-sm text-muted-foreground sm:text-base">
					Ask FlowPilot to create apps, find packages, or navigate Flow-Like.
				</p>
			</div>

			<div className="relative flex min-h-36 w-full max-w-2xl items-center justify-center">
				{suggestions.map((suggestion, index) => {
					const positions = [
						"left-0 top-1",
						"right-0 top-2",
						"left-4 bottom-0",
						"right-4 bottom-0",
					];
					return (
						<button
							type="button"
							key={suggestion}
							onClick={() => {
								setPrompt(suggestion);
								setOpen(true);
							}}
							className={cn(
								"absolute hidden items-center gap-1.5 rounded-full border border-border/50 bg-background/85 px-3 py-1.5 text-[11px] text-muted-foreground shadow-sm backdrop-blur-md transition hover:-translate-y-0.5 hover:text-foreground md:flex",
								positions[index],
							)}
						>
							<Sparkles className="size-3" /> {suggestion}
						</button>
					);
				})}

				<div
					className={cn(
						"relative z-10 overflow-hidden p-[2px] shadow-[0_18px_70px_-28px_rgba(124,58,237,0.75)] transition-all duration-500",
						open
							? "h-32 w-[min(100%,36rem)] rounded-[2rem]"
							: "h-20 w-64 cursor-pointer rounded-full",
					)}
					style={{
						background:
							"radial-gradient(circle at 20% 20%, rgba(255,255,255,.95), transparent 26%), conic-gradient(from 155deg, #22d3ee, #8b5cf6, #fb7185, #fbbf24, #34d399, #22d3ee)",
					}}
					onClick={() => setOpen(true)}
					onKeyDown={(event) => {
						if (event.key === "Enter" || event.key === " ") setOpen(true);
					}}
					role={open ? undefined : "button"}
					tabIndex={open ? undefined : 0}
				>
					<div className="relative flex size-full flex-col justify-between rounded-[calc(2rem-2px)] bg-background/90 p-3 backdrop-blur-xl">
						{open ? (
							<>
								<textarea
									value={prompt}
									onChange={(event) => setPrompt(event.target.value)}
									placeholder="Ask FlowPilot anything, or describe what you want to build…"
									aria-label="Ask FlowPilot"
									className="min-h-12 w-full resize-none border-0 bg-transparent px-2 text-[15px] outline-none placeholder:text-muted-foreground"
								/>
								<div className="flex items-center justify-between">
									<div className="flex items-center gap-2">
										<span className="flex size-7 items-center justify-center rounded-full bg-primary/10 text-primary">
											<Sparkles className="size-4" />
										</span>
										<Badge variant="outline">Profile · Auto</Badge>
									</div>
									<div className="flex items-center gap-1.5">
										<IconButton
											label="Attach images"
											className="size-8 rounded-full border-0 bg-transparent shadow-none"
										>
											<Paperclip className="size-[18px]" />
										</IconButton>
										<IconButton
											label="Send"
											className="size-9 rounded-full border-0 bg-foreground text-background shadow-none"
											onClick={() => setPrompt("")}
										>
											<ArrowUp className="size-5" />
										</IconButton>
										<span className="flex size-8 items-center justify-center rounded-full text-muted-foreground/40">
											<AudioLines className="size-[18px]" />
										</span>
									</div>
								</div>
							</>
						) : (
							<div className="flex size-full items-center justify-center gap-2 text-sm font-medium">
								<Sparkles className="size-4" /> Click to ask FlowPilot
							</div>
						)}
					</div>
				</div>
			</div>

			<div className="flex max-w-full gap-2 overflow-x-auto pb-1 md:hidden">
				{suggestions.map((suggestion) => (
					<button
						type="button"
						key={`mobile-${suggestion}`}
						onClick={() => {
							setPrompt(suggestion);
							setOpen(true);
						}}
						className="shrink-0 rounded-full border border-border bg-card px-3 py-1.5 text-[11px] text-muted-foreground"
					>
						{suggestion}
					</button>
				))}
			</div>
		</div>
	);
}

function HomeSurface() {
	return (
		<div className="h-[43rem] overflow-y-auto">
			<FlowPilotHero />
			<div className="w-full bg-gradient-to-b from-background/0 via-background/90 to-background px-5 pb-12 pt-2">
				<SectionHeader
					title="Browse by category"
					subtitle="Find the right app for every job."
					action="Explore all"
				/>
				<div className="flex snap-x gap-3 overflow-x-auto pb-2">
					{CATEGORY_FIXTURES.map((category) => (
						<button
							type="button"
							key={category.label}
							className="group flex shrink-0 snap-start items-center gap-2.5 rounded-full border border-border/40 bg-card/70 py-2 pl-2 pr-4 backdrop-blur-sm transition-all hover:-translate-y-0.5 hover:border-primary/30 hover:shadow-md"
						>
							<span
								className="flex size-7 items-center justify-center rounded-full"
								style={{
									backgroundColor: `color-mix(in oklab, ${category.color} 16%, transparent)`,
								}}
							>
								<category.icon
									className="size-3.5"
									style={{ color: category.color }}
								/>
							</span>
							<span className="whitespace-nowrap text-sm font-medium">
								{category.label}
							</span>
						</button>
					))}
				</div>

				<div className="mt-10">
					<SectionHeader
						title="Top charts"
						subtitle="The apps people are building with right now."
						action="View all"
					/>
					<div className="grid gap-x-8 gap-y-3 md:grid-cols-2">
						{APP_FIXTURES.map((app, index) => (
							<div
								key={`ranked-${app.id}`}
								className="flex min-w-0 items-center gap-3"
							>
								<span className="w-7 shrink-0 text-center text-lg font-bold tabular-nums text-muted-foreground/40">
									{index + 1}
								</span>
								<MiniAppCard app={app} compact />
							</div>
						))}
					</div>
				</div>
			</div>
		</div>
	);
}

function ExploreHeader({
	tab,
	onTabChange,
}: Readonly<{ tab: ExploreTab; onTabChange: (tab: ExploreTab) => void }>) {
	return (
		<div className="flex flex-wrap items-end justify-between gap-3">
			<div className="min-w-0 space-y-1">
				<h3 className="text-2xl font-bold tracking-tight">Explore</h3>
				<p className="text-sm text-muted-foreground">
					{tab === "apps"
						? "Community apps, ready to use or fork."
						: "Discover and install WASM node packages."}
				</p>
			</div>
			<nav
				aria-label="Explore sections"
				className="inline-flex items-center rounded-full border border-border/40 bg-muted/30 p-1"
			>
				{[
					{ key: "apps" as const, label: "Apps", icon: LayoutGrid },
					{ key: "packages" as const, label: "Packages", icon: Package },
				].map((item) => (
					<button
						type="button"
						key={item.key}
						aria-current={tab === item.key ? "page" : undefined}
						onClick={() => onTabChange(item.key)}
						className={cn(
							"flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-sm font-medium transition-colors",
							tab === item.key
								? "bg-background text-foreground shadow-sm"
								: "text-muted-foreground hover:text-foreground",
						)}
					>
						<item.icon className="size-3.5" /> {item.label}
					</button>
				))}
			</nav>
		</div>
	);
}

function PackageCard({ pkg }: Readonly<{ pkg: PackageFixture }>) {
	return (
		<button
			type="button"
			className="group relative flex h-56 w-full flex-col overflow-hidden rounded-2xl border border-border/50 text-left shadow-sm transition-all hover:-translate-y-1 hover:border-primary/40 hover:shadow-xl"
		>
			<span
				className={cn(
					"absolute inset-0 bg-gradient-to-br transition-transform duration-500 group-hover:scale-105",
					pkg.color,
				)}
			/>
			<span className="absolute inset-0 bg-gradient-to-b from-black/25 via-black/55 to-black/85 backdrop-blur-[2px]" />
			<span className="relative z-10 flex h-full flex-col p-4">
				<span className="flex items-center gap-3">
					<span className="flex size-11 shrink-0 items-center justify-center rounded-xl border border-white/20 bg-white/10 font-mono text-xs font-semibold text-white shadow-lg backdrop-blur-md">
						{pkg.name.split("/")[1].slice(0, 2).toUpperCase()}
					</span>
					<span className="min-w-0 flex-1">
						<span className="block truncate text-[10px] font-semibold uppercase tracking-wider text-white/60">
							{pkg.category}
						</span>
						<span className="flex items-center gap-1.5">
							<strong className="truncate font-mono text-sm text-white">
								{pkg.name}
							</strong>
							<Shield className="size-3.5 shrink-0 text-sky-400" />
						</span>
					</span>
					<span className="rounded-md border border-white/20 bg-white/10 px-2 py-0.5 font-mono text-[10px] text-white/85">
						v{pkg.version}
					</span>
				</span>
				<span className="mt-3 line-clamp-2 text-xs leading-relaxed text-white/70">
					{pkg.description}
				</span>
				<span className="mt-auto grid grid-cols-3 gap-2">
					{[
						{ value: pkg.installs, label: "Installs" },
						{ value: `★ ${pkg.rating}`, label: "Rating" },
						{ value: "Free", label: "Price" },
					].map((stat) => (
						<span
							key={stat.label}
							className="rounded-xl border border-white/10 bg-white/10 px-2 py-2 text-center backdrop-blur-sm"
						>
							<strong className="block font-mono text-sm tabular-nums text-white">
								{stat.value}
							</strong>
							<span className="mt-0.5 block text-[9px] uppercase tracking-wider text-white/55">
								{stat.label}
							</span>
						</span>
					))}
				</span>
			</span>
		</button>
	);
}

function ExploreSurfaceContent() {
	const [tab, setTab] = useState<ExploreTab>("apps");
	const [query, setQuery] = useState("");
	const [category, setCategory] = useState<string | null>(null);
	const [verifiedOnly, setVerifiedOnly] = useState(false);
	const normalized = query.trim().toLocaleLowerCase();
	const visibleApps = APP_FIXTURES.filter(
		(app) =>
			(!category || app.category === category) &&
			(!normalized ||
				`${app.name} ${app.description}`
					.toLocaleLowerCase()
					.includes(normalized)),
	);
	const visiblePackages = PACKAGE_FIXTURES.filter(
		(pkg) =>
			!normalized ||
			`${pkg.name} ${pkg.description}`.toLocaleLowerCase().includes(normalized),
	);

	return (
		<div className="h-[43rem] overflow-y-auto p-5 sm:p-6">
			<ExploreHeader
				tab={tab}
				onTabChange={(next) => {
					setTab(next);
					setQuery("");
				}}
			/>
			{tab === "apps" ? (
				<>
					<div className="mt-5 flex items-center gap-2">
						<label className="relative min-w-0 max-w-lg flex-1">
							<span className="sr-only">Search community apps</span>
							<Search className="pointer-events-none absolute left-4 top-1/2 size-4 -translate-y-1/2 text-muted-foreground/40" />
							<input
								value={query}
								onChange={(event) => setQuery(event.target.value)}
								placeholder="Search community apps…"
								className="h-10 w-full rounded-full border border-transparent bg-muted/30 pl-11 pr-10 text-sm outline-none transition-all focus:border-border/40 focus:bg-muted/50"
							/>
							{query ? (
								<button
									type="button"
									aria-label="Clear search"
									onClick={() => setQuery("")}
									className="absolute right-4 top-1/2 -translate-y-1/2 text-muted-foreground/40 hover:text-foreground"
								>
									<X className="size-4" />
								</button>
							) : null}
						</label>
						<select
							aria-label="Sort results"
							className="h-10 rounded-full border border-border/40 bg-muted/30 px-3 text-sm outline-none"
						>
							<option>Most popular</option>
							<option>Newest first</option>
							<option>Best rated</option>
						</select>
					</div>
					<div className="mt-4 flex gap-1.5 overflow-x-auto pb-1">
						{CATEGORY_FIXTURES.map((item) => {
							const selected = category === item.label;
							return (
								<button
									type="button"
									key={`filter-${item.label}`}
									aria-pressed={selected}
									onClick={() => setCategory(selected ? null : item.label)}
									className={cn(
										"flex shrink-0 items-center gap-1.5 rounded-full px-3 py-1.5 text-xs transition-all",
										selected
											? "bg-foreground/10 text-foreground ring-1 ring-foreground/20"
											: "bg-muted/20 text-muted-foreground/70 hover:bg-muted/40 hover:text-foreground/80",
									)}
								>
									<item.icon
										className="size-3"
										style={{ color: item.color, opacity: selected ? 1 : 0.7 }}
									/>{" "}
									{item.label}
									{selected ? <X className="size-3" /> : null}
								</button>
							);
						})}
					</div>
					<div className="mt-7">
						<div className="mb-3 flex items-center justify-between gap-3">
							<div className="flex min-w-0 items-center gap-2">
								<span className="size-2 rounded-full bg-primary" />
								<h4 className="truncate text-base font-bold tracking-tight">
									{category ?? "Featured apps"}
								</h4>
							</div>
							<button
								type="button"
								className="flex items-center gap-1 text-sm font-medium text-muted-foreground hover:text-foreground"
							>
								See all <ArrowRight className="size-3.5" />
							</button>
						</div>
						{visibleApps.length ? (
							<div className="flex snap-x gap-4 overflow-x-auto pb-3">
								{visibleApps.map((app) => (
									<MiniAppCard key={`explore-${app.id}`} app={app} />
								))}
							</div>
						) : (
							<div className="rounded-xl border border-dashed border-border p-10 text-center text-sm text-muted-foreground">
								No apps found. Try another search or category.
							</div>
						)}
					</div>
				</>
			) : (
				<>
					<div className="mt-6 flex flex-col gap-4 sm:flex-row">
						<label className="relative min-w-0 flex-1">
							<span className="sr-only">Search packages</span>
							<Search className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
							<input
								value={query}
								onChange={(event) => setQuery(event.target.value)}
								placeholder="Search packages..."
								className="h-10 w-full rounded-full border border-border/20 bg-muted/30 pl-10 pr-3 text-sm outline-none focus:ring-2 focus:ring-primary/40"
							/>
						</label>
						<div className="flex gap-2">
							<select
								aria-label="Sort packages"
								className="h-10 rounded-md border border-border bg-background px-3 text-sm"
							>
								<option>Most Downloads</option>
								<option>Recently Updated</option>
								<option>Newest</option>
							</select>
							<button
								type="button"
								aria-pressed={verifiedOnly}
								onClick={() => setVerifiedOnly((value) => !value)}
								className={cn(
									"flex items-center gap-2 rounded-full border px-4 py-2 text-sm transition-colors",
									verifiedOnly
										? "border-primary bg-primary text-primary-foreground"
										: "border-border/30 text-muted-foreground hover:bg-muted/30",
								)}
							>
								<Shield className="size-4" /> Verified
							</button>
						</div>
					</div>
					<p className="mt-5 text-xs text-muted-foreground/60">
						{visiblePackages.length} packages found
					</p>
					<div className="mt-4 grid gap-3 md:grid-cols-2">
						{visiblePackages.map((pkg) => (
							<PackageCard key={pkg.name} pkg={pkg} />
						))}
					</div>
				</>
			)}
		</div>
	);
}

export function HomeExploreDemo() {
	const [surface, setSurface] = useState<ExploreSurface>("home");
	const navigation: Array<{
		key: string;
		label: string;
		icon: LucideIcon;
		surface?: ExploreSurface;
	}> = [
		{ key: "home", label: "Home", icon: Home, surface: "home" },
		{ key: "flowpilot", label: "FlowPilot", icon: Sparkles },
		{ key: "explore", label: "Explore", icon: Store, surface: "explore" },
		{ key: "models", label: "Explore Models", icon: Code2 },
		{ key: "library", label: "My Apps", icon: AppWindow },
	];

	return (
		<ProductDemoFrame source="Home · FlowPilot hero · Explore hub">
			<div className="overflow-hidden rounded-2xl border border-border bg-background shadow-sm">
				<div className="grid grid-cols-[3.5rem_minmax(0,1fr)] sm:grid-cols-[11rem_minmax(0,1fr)]">
					<aside className="border-r border-border bg-muted/10 p-2 sm:p-3">
						<div className="mb-5 hidden rounded-lg border border-border bg-card p-2 sm:flex sm:items-center sm:gap-2">
							<span className="flex size-8 items-center justify-center rounded-lg bg-primary text-xs font-bold text-primary-foreground">
								FL
							</span>
							<span className="min-w-0">
								<strong className="block truncate text-sm">Flow-Like</strong>
								<span className="block truncate text-[10px] text-muted-foreground">
									Local profile
								</span>
							</span>
						</div>
						<p className="mb-2 hidden px-2 text-xs font-medium text-muted-foreground sm:block">
							Navigation
						</p>
						<nav aria-label="Demo navigation" className="space-y-1">
							{navigation.map((item) => {
								const active = item.surface === surface;
								return (
									<button
										type="button"
										key={item.key}
										title={item.label}
										onClick={
											item.surface
												? () => setSurface(item.surface as ExploreSurface)
												: undefined
										}
										className={cn(
											"flex w-full items-center justify-center gap-2 rounded-md px-2 py-2 text-sm transition-colors sm:justify-start",
											active
												? "border border-border bg-background font-medium shadow-xs"
												: "text-muted-foreground hover:bg-muted hover:text-foreground",
										)}
									>
										<item.icon className="size-4 shrink-0" />
										<span className="hidden truncate sm:inline">
											{item.label}
										</span>
									</button>
								);
							})}
						</nav>
					</aside>
					<main className="min-w-0">
						{surface === "home" ? <HomeSurface /> : <ExploreSurfaceContent />}
					</main>
				</div>
			</div>
		</ProductDemoFrame>
	);
}
