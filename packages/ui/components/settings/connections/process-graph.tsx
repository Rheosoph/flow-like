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
	MiniMap,
	type Node,
	type NodeProps,
	Position,
	ReactFlow,
	ReactFlowProvider,
	getBezierPath,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { formatDistanceToNow } from "date-fns";
import {
	ArrowRight,
	Blocks,
	BookOpen,
	CheckCircle2,
	ChevronRight,
	Clock,
	Database,
	ExternalLink,
	Eye,
	FileText,
	GitBranch,
	Layers,
	LayoutGrid,
	LayoutTemplate,
	Lock,
	type LucideIcon,
	Maximize2,
	Minimize2,
	Pencil,
	PlayCircle,
	Plus,
	RefreshCw,
	Search,
	Shield,
	Sparkles,
	StickyNote,
	Table2,
	Trash2,
	Workflow,
	X,
	XCircle,
	Zap,
} from "lucide-react";
import { useTheme } from "next-themes";
import {
	type ReactNode,
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import type {
	IAppContentStats,
	IProcessCase,
	IProcessFlow,
	IProcessGraphNode,
	IProcessGraphResponse,
	IProcessNote,
} from "../../..";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	EmptyState,
	Input,
	RolePermissions,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	Textarea,
	cn,
	useBackend,
	useInvoke,
} from "../../..";
import {
	ALL_PERMISSIONS,
	getPermissionLabel,
} from "../roles/permission-groups";
import {
	ACCESS_LABEL,
	AccessIcon,
	type AccessLevel,
	type ConnectionCapabilities,
	deriveConnectionCapabilities,
	hasConnectionCapabilities,
} from "./capabilities";

const MAX_NOTE_LENGTH = 4096;
const X_GAP = 440;
const Y_GAP = 130;
const TIME_WINDOWS = [7, 30, 90, 365] as const;

export interface ProcessGraphProps {
	data?: IProcessGraphResponse;
	/** Reconstructed end-to-end process cases (from the correlation spine). */
	cases?: IProcessCase[];
	casesLoading?: boolean;
	casesError?: boolean;
	isLoading?: boolean;
	days: number;
	onDaysChange: (days: number) => void;
	onRefresh: () => void;
	/** Notes are always written against the app id of the annotated node. */
	onCreateNote?: (targetAppId: string, content: string) => Promise<void>;
	onUpdateNote?: (
		targetAppId: string,
		noteId: string,
		content: string,
	) => Promise<void>;
	onDeleteNote?: (targetAppId: string, noteId: string) => Promise<void>;
}

type ProcessGraphFlowNode = Node<
	{ app: IProcessGraphNode; dimmed?: boolean },
	"processApp"
>;

function appLabel(app: IProcessGraphNode): string {
	if (app.unknown) return "Unknown App";
	return app.name ?? app.id;
}

/** "FoodAndDrink" -> "Food And Drink" for display. */
function prettifyCategory(category: string): string {
	return category.replace(/([a-z])([A-Z])/g, "$1 $2");
}

/** Formats a millisecond duration compactly (e.g. "820ms", "3.4s", "2m"). */
function formatDuration(ms: number): string {
	if (ms < 1000) return `${Math.round(ms)}ms`;
	if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
	return `${Math.round(ms / 60_000)}m`;
}

const ProcessAppNode = memo(
	({ data, selected }: NodeProps<ProcessGraphFlowNode>) => {
		const app = data.app;

		return (
			<div
				title={app.unknown ? "Unknown app" : appLabel(app)}
				className={cn(
					"w-52 cursor-pointer rounded-lg border bg-card p-3 shadow-sm transition-all hover:shadow-md",
					app.is_current && "ring-2 ring-primary",
					app.unknown && "border-dashed",
					selected && !app.is_current && "ring-1 ring-ring",
					data.dimmed && "opacity-30",
				)}
			>
				<Handle
					type="target"
					position={Position.Left}
					isConnectable={false}
					className="!h-2 !w-2 !border-0 !bg-muted-foreground"
				/>
				<div className="flex min-w-0 items-center gap-2">
					<Avatar className="h-8 w-8 rounded-md">
						<AvatarImage
							src={app.icon ?? undefined}
							alt={`${appLabel(app)} icon`}
						/>
						<AvatarFallback className="rounded-md bg-primary/10">
							<Blocks className="h-4 w-4 text-primary" />
						</AvatarFallback>
					</Avatar>
					<div className="min-w-0 flex-1">
						<p
							className={cn(
								"truncate text-sm font-medium",
								app.unknown && "italic text-muted-foreground",
							)}
						>
							{appLabel(app)}
						</p>
						{app.is_current && (
							<p className="text-[10px] font-medium text-primary">This app</p>
						)}
					</div>
					{app.notes.length > 0 && (
						<Badge
							variant="secondary"
							className="shrink-0 gap-1 px-1.5 text-[10px]"
						>
							<StickyNote className="h-3 w-3" />
							{app.notes.length}
						</Badge>
					)}
				</div>
				<Handle
					type="source"
					position={Position.Right}
					isConnectable={false}
					className="!h-2 !w-2 !border-0 !bg-muted-foreground"
				/>
			</div>
		);
	},
);
ProcessAppNode.displayName = "ProcessAppNode";

const nodeTypes = { processApp: ProcessAppNode };

function computeLayout(
	nodes: IProcessGraphNode[],
	pairs: [string, string][],
): Map<string, { x: number; y: number }> {
	const ids = new Set(nodes.map((n) => n.id));
	const outgoing = new Map<string, string[]>();
	const inDegree = new Map<string, number>();
	for (const id of ids) {
		outgoing.set(id, []);
		inDegree.set(id, 0);
	}
	for (const [source, target] of pairs) {
		if (!ids.has(source) || !ids.has(target) || source === target) continue;
		outgoing.get(source)?.push(target);
		inDegree.set(target, (inDegree.get(target) ?? 0) + 1);
	}

	let roots = nodes.filter((n) => (inDegree.get(n.id) ?? 0) === 0);
	if (roots.length === 0) {
		const fallback = nodes.find((n) => n.is_current) ?? nodes[0];
		roots = fallback ? [fallback] : [];
	}

	const depth = new Map<string, number>();
	const queue: string[] = [];
	for (const root of roots) {
		depth.set(root.id, 0);
		queue.push(root.id);
	}
	while (queue.length > 0) {
		const current = queue.shift() as string;
		const currentDepth = depth.get(current) ?? 0;
		for (const next of outgoing.get(current) ?? []) {
			if (depth.has(next)) continue;
			depth.set(next, currentDepth + 1);
			queue.push(next);
		}
	}
	for (const node of nodes) {
		if (!depth.has(node.id)) {
			depth.set(node.id, 0);
			const stack = [node.id];
			while (stack.length > 0) {
				const current = stack.pop() as string;
				const currentDepth = depth.get(current) ?? 0;
				for (const next of outgoing.get(current) ?? []) {
					if (depth.has(next)) continue;
					depth.set(next, currentDepth + 1);
					stack.push(next);
				}
			}
		}
	}

	const layers = new Map<number, string[]>();
	for (const node of nodes) {
		const d = depth.get(node.id) ?? 0;
		const layer = layers.get(d) ?? [];
		layer.push(node.id);
		layers.set(d, layer);
	}

	// Barycenter ordering: repeatedly reorder each layer by the mean position of
	// its predecessors in the previous layer to reduce edge crossings.
	const incoming = new Map<string, string[]>();
	for (const [source, targets] of outgoing) {
		for (const target of targets) {
			const list = incoming.get(target) ?? [];
			list.push(source);
			incoming.set(target, list);
		}
	}
	const maxDepth = layers.size > 0 ? Math.max(...layers.keys()) : 0;
	const orderOf = (d: number, id: string) => layers.get(d)?.indexOf(id) ?? 0;
	for (let sweep = 0; sweep < 4; sweep++) {
		for (let d = 1; d <= maxDepth; d++) {
			const layer = layers.get(d);
			if (!layer || layer.length < 2) continue;
			const bary = new Map<string, number>();
			for (const id of layer) {
				const preds = (incoming.get(id) ?? []).filter(
					(p) => depth.get(p) === d - 1,
				);
				bary.set(
					id,
					preds.length > 0
						? preds.reduce((sum, p) => sum + orderOf(d - 1, p), 0) /
								preds.length
						: orderOf(d, id),
				);
			}
			layer.sort((a, b) => (bary.get(a) ?? 0) - (bary.get(b) ?? 0));
		}
	}

	// Center each layer vertically around a shared midline.
	const maxCount = Math.max(
		1,
		...[...layers.values()].map((layer) => layer.length),
	);
	const positions = new Map<string, { x: number; y: number }>();
	for (const [d, layer] of layers) {
		const yOffset = ((maxCount - layer.length) * Y_GAP) / 2;
		layer.forEach((id, index) => {
			positions.set(id, { x: d * X_GAP, y: yOffset + index * Y_GAP });
		});
	}
	return positions;
}

interface HopInfo {
	source: string;
	target: string;
	runCount?: number;
	roleName?: string | null;
	permissions?: number | null;
	status?: string;
	pending: boolean;
}

interface ConnectionEdgeData extends Record<string, unknown> {
	source: string;
	target: string;
	roleName?: string | null;
	permissions?: number | null;
	capabilities: ConnectionCapabilities;
	runCount?: number;
	pending: boolean;
	observed: boolean;
	/** Perpendicular label shift so reciprocal edges don't overlap. */
	labelOffset: number;
	dimmed?: boolean;
}

/** Every permission label the role grants, for the edge detail panel. */
function grantedPermissionLabels(bits?: number | null): string[] {
	if (!bits) return [];
	const perms = new RolePermissions(BigInt(bits));
	return ALL_PERMISSIONS.filter((permission) => perms.contains(permission))
		.map((permission) => getPermissionLabel(permission))
		.filter((label): label is string => Boolean(label));
}

function buildEdges(data: IProcessGraphResponse): Edge[] {
	const hops = new Map<string, HopInfo>();

	for (const edge of data.edges) {
		hops.set(`${edge.source} ${edge.target}`, {
			source: edge.source,
			target: edge.target,
			roleName: edge.role_name,
			permissions: edge.role_permissions,
			status: edge.status,
			pending: edge.status === "PENDING",
		});
	}

	for (const flow of data.flows) {
		for (let i = 0; i < flow.path.length - 1; i++) {
			const source = flow.path[i];
			const target = flow.path[i + 1];
			const key = `${source} ${target}`;
			const existing = hops.get(key);
			if (existing) {
				existing.runCount = (existing.runCount ?? 0) + flow.run_count;
			} else {
				hops.set(key, {
					source,
					target,
					runCount: flow.run_count,
					pending: false,
				});
			}
		}
	}

	return Array.from(hops.values()).map((hop) => {
		const observed = hop.runCount !== undefined;
		const stroke = observed ? "var(--primary)" : "var(--muted-foreground)";
		// Reciprocal edges share endpoints — shift their labels apart.
		const reciprocal = hops.has(`${hop.target} ${hop.source}`);
		const labelOffset = reciprocal ? (hop.source < hop.target ? -14 : 14) : 0;

		return {
			id: `edge-${hop.source}-${hop.target}`,
			source: hop.source,
			target: hop.target,
			type: "connection",
			animated: observed,
			data: {
				source: hop.source,
				target: hop.target,
				roleName: hop.roleName,
				permissions: hop.permissions,
				capabilities: deriveConnectionCapabilities(hop.permissions),
				runCount: hop.runCount,
				pending: hop.pending,
				observed,
				labelOffset,
			} satisfies ConnectionEdgeData,
			style: {
				stroke,
				strokeWidth: observed ? 2 : 1.5,
				...(hop.pending && !observed
					? { strokeDasharray: "6 4", opacity: 0.6 }
					: {}),
			},
			markerEnd: {
				type: MarkerType.ArrowClosed,
				color: stroke,
			},
		} satisfies Edge;
	});
}

const ConnectionEdge = memo((props: EdgeProps) => {
	const {
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
	} = props;
	const [edgePath, labelX, labelY] = getBezierPath({
		sourceX,
		sourceY,
		sourcePosition,
		targetX,
		targetY,
		targetPosition,
	});

	const edgeData = data as ConnectionEdgeData | undefined;
	const roleName = edgeData?.roleName;
	const capabilities = edgeData?.capabilities ?? { events: false };
	const runCount = edgeData?.runCount;
	const showCaps = hasConnectionCapabilities(capabilities);
	const dimmed = edgeData?.dimmed ?? false;
	const offset = edgeData?.labelOffset ?? 0;
	const hasMeta = Boolean(roleName) || showCaps || runCount !== undefined;

	const edgeStyle = {
		...style,
		opacity: dimmed ? 0.1 : (style?.opacity ?? 1),
	};

	return (
		<>
			<BaseEdge
				id={id}
				path={edgePath}
				markerEnd={markerEnd}
				style={edgeStyle}
				interactionWidth={24}
			/>
			{hasMeta && (
				<EdgeLabelRenderer>
					<div
						style={{
							transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY + offset}px)`,
							opacity: dimmed ? 0.2 : 1,
						}}
						className="pointer-events-none absolute flex flex-col items-center gap-0.5 rounded-md border bg-background/90 px-1.5 py-1 text-center shadow-sm backdrop-blur-sm"
					>
						{roleName && (
							<span className="max-w-28 truncate text-[10px] font-medium leading-none text-foreground">
								{roleName}
							</span>
						)}
						{showCaps && (
							<div className="flex items-center gap-1">
								{capabilities.events && (
									<Zap
										className="h-3 w-3 text-muted-foreground"
										aria-label="Executes events"
									/>
								)}
								{capabilities.database && (
									<Database
										className={cn(
											"h-3 w-3",
											capabilities.database === "readwrite"
												? "text-foreground"
												: "text-muted-foreground",
										)}
										aria-label={`Database ${ACCESS_LABEL[capabilities.database]}`}
									/>
								)}
								{capabilities.files && (
									<FileText
										className={cn(
											"h-3 w-3",
											capabilities.files === "readwrite"
												? "text-foreground"
												: "text-muted-foreground",
										)}
										aria-label={`Files ${ACCESS_LABEL[capabilities.files]}`}
									/>
								)}
							</div>
						)}
						{runCount !== undefined && (
							<span className="text-[9px] leading-none text-muted-foreground">
								{runCount} {runCount === 1 ? "run" : "runs"}
							</span>
						)}
					</div>
				</EdgeLabelRenderer>
			)}
		</>
	);
});
ConnectionEdge.displayName = "ConnectionEdge";

const edgeTypes = { connection: ConnectionEdge };

function NoteAuthor({ userId }: Readonly<{ userId?: string | null }>) {
	const backend = useBackend();
	const user = useInvoke(
		backend.userState.lookupUser,
		backend.userState,
		[userId ?? ""],
		Boolean(userId),
	);

	const label =
		user.data?.name ??
		user.data?.preferred_username ??
		user.data?.username ??
		user.data?.email ??
		userId ??
		"Unknown author";

	return <span className="truncate font-medium">{label}</span>;
}

interface NoteItemProps {
	note: IProcessNote;
	canAnnotate: boolean;
	onUpdate?: (noteId: string, content: string) => Promise<void>;
	onDelete?: (noteId: string) => Promise<void>;
}

function NoteItem({
	note,
	canAnnotate,
	onUpdate,
	onDelete,
}: Readonly<NoteItemProps>) {
	const [isEditing, setIsEditing] = useState(false);
	const [draft, setDraft] = useState(note.content);
	const [isBusy, setIsBusy] = useState(false);

	const handleSave = useCallback(async () => {
		if (!onUpdate || !draft.trim()) return;
		try {
			setIsBusy(true);
			await onUpdate(note.id, draft.trim());
			setIsEditing(false);
			toast.success("Note updated");
		} catch (error) {
			console.error(error);
			toast.error(
				error instanceof Error && error.message
					? error.message
					: "Failed to update note",
			);
		} finally {
			setIsBusy(false);
		}
	}, [draft, note.id, onUpdate]);

	const handleDelete = useCallback(async () => {
		if (!onDelete) return;
		try {
			setIsBusy(true);
			await onDelete(note.id);
			toast.success("Note deleted");
		} catch (error) {
			console.error(error);
			toast.error(
				error instanceof Error && error.message
					? error.message
					: "Failed to delete note",
			);
		} finally {
			setIsBusy(false);
		}
	}, [note.id, onDelete]);

	return (
		<div className="rounded-md border bg-muted/30 p-2.5">
			<div className="flex items-center gap-2 text-xs text-muted-foreground">
				<NoteAuthor userId={note.author_user_id} />
				<span className="shrink-0">
					{formatDistanceToNow(new Date(note.updated_at * 1000), {
						addSuffix: true,
					})}
				</span>
				{canAnnotate && (
					<div className="ml-auto flex shrink-0 items-center gap-0.5">
						<Button
							variant="ghost"
							size="icon"
							className="h-6 w-6"
							disabled={isBusy}
							onClick={() => {
								setDraft(note.content);
								setIsEditing((editing) => !editing);
							}}
						>
							<Pencil className="h-3 w-3" />
						</Button>
						<AlertDialog>
							<AlertDialogTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-6 w-6 text-destructive hover:text-destructive"
									disabled={isBusy}
								>
									<Trash2 className="h-3 w-3" />
								</Button>
							</AlertDialogTrigger>
							<AlertDialogContent>
								<AlertDialogHeader>
									<AlertDialogTitle>Delete Note</AlertDialogTitle>
									<AlertDialogDescription>
										Are you sure you want to delete this process note? This
										action cannot be undone.
									</AlertDialogDescription>
								</AlertDialogHeader>
								<AlertDialogFooter>
									<AlertDialogCancel>Cancel</AlertDialogCancel>
									<AlertDialogAction
										onClick={handleDelete}
										className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
									>
										Delete
									</AlertDialogAction>
								</AlertDialogFooter>
							</AlertDialogContent>
						</AlertDialog>
					</div>
				)}
			</div>
			{isEditing ? (
				<div className="mt-2 space-y-2">
					<Textarea
						value={draft}
						maxLength={MAX_NOTE_LENGTH}
						onChange={(e) => setDraft(e.target.value)}
						className="min-h-15 resize-none text-sm"
					/>
					<div className="flex justify-end gap-2">
						<Button
							variant="outline"
							size="sm"
							onClick={() => setIsEditing(false)}
						>
							Cancel
						</Button>
						<Button
							size="sm"
							onClick={handleSave}
							disabled={isBusy || !draft.trim()}
						>
							{isBusy ? "Saving..." : "Save"}
						</Button>
					</div>
				</div>
			) : (
				<p className="mt-1.5 whitespace-pre-wrap text-sm leading-relaxed">
					{note.content}
				</p>
			)}
		</div>
	);
}

const CONTENT_ITEMS: ReadonlyArray<{
	key: keyof IAppContentStats;
	label: string;
	icon: LucideIcon;
}> = [
	{ key: "events", label: "Events", icon: Zap },
	{ key: "pages", label: "Pages", icon: Layers },
	{ key: "templates", label: "Templates", icon: LayoutTemplate },
	{ key: "widgets", label: "Widgets", icon: LayoutGrid },
];

function PanelSection({
	icon: Icon,
	title,
	count,
	children,
}: Readonly<{
	icon: LucideIcon;
	title: string;
	count?: number;
	children: ReactNode;
}>) {
	return (
		<section className="space-y-2">
			<div className="flex items-center gap-1.5">
				<Icon className="h-3.5 w-3.5 text-muted-foreground" />
				<h4 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
					{title}
				</h4>
				{count !== undefined && count > 0 && (
					<Badge variant="secondary" className="h-4 px-1.5 text-[10px]">
						{count}
					</Badge>
				)}
			</div>
			{children}
		</section>
	);
}

function ContentStats({ content }: Readonly<{ content: IAppContentStats }>) {
	const items = CONTENT_ITEMS.filter((item) => content[item.key] > 0);
	if (items.length === 0) return null;
	return (
		<PanelSection icon={Blocks} title="Contents">
			<div className="flex flex-wrap gap-1.5">
				{items.map(({ key, label, icon: Icon }) => (
					<div
						key={label}
						className="flex items-center gap-1.5 rounded-md border bg-muted/30 px-2 py-1"
					>
						<Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
						<span className="text-xs font-semibold">{content[key]}</span>
						<span className="text-[11px] text-muted-foreground">{label}</span>
					</div>
				))}
			</div>
		</PanelSection>
	);
}

/**
 * Lazily lists the app's data tables (lancedb tables) when its panel is opened.
 * Requires DB read access on the app; on a permission error the section is
 * hidden rather than surfacing a failure.
 */
function AppTables({ appId }: Readonly<{ appId: string }>) {
	const backend = useBackend();
	const tables = useInvoke(backend.dbState.listTables, backend.dbState, [
		appId,
	]);

	if (tables.error) return null;

	return (
		<PanelSection icon={Database} title="Tables" count={tables.data?.length}>
			{tables.isLoading ? (
				<div className="space-y-1.5">
					<Skeleton className="h-8 w-full" />
					<Skeleton className="h-8 w-2/3" />
				</div>
			) : tables.data && tables.data.length > 0 ? (
				<div className="space-y-1">
					{tables.data.map((table) => (
						<div
							key={table}
							className="flex items-center gap-2 rounded-md border bg-muted/30 px-2 py-1.5 text-xs"
						>
							<Table2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
							<span className="truncate font-medium">{table}</span>
						</div>
					))}
				</div>
			) : (
				<p className="text-xs text-muted-foreground">No data tables yet.</p>
			)}
		</PanelSection>
	);
}

function NoAccessPlaceholder() {
	return (
		<div className="flex flex-col items-center gap-2 rounded-lg border border-dashed py-8 text-center">
			<div className="flex h-10 w-10 items-center justify-center rounded-full bg-muted">
				<Lock className="h-5 w-5 text-muted-foreground" />
			</div>
			<p className="text-sm font-medium">No access to this app</p>
			<p className="max-w-52 text-xs text-muted-foreground">
				You&apos;re not a member of this app, so its details, contents, and
				process notes are hidden.
			</p>
		</div>
	);
}

interface NodeDetailsPanelProps {
	app: IProcessGraphNode;
	onClose: () => void;
	onCreateNote?: ProcessGraphProps["onCreateNote"];
	onUpdateNote?: ProcessGraphProps["onUpdateNote"];
	onDeleteNote?: ProcessGraphProps["onDeleteNote"];
}

function NodeDetailsPanel({
	app,
	onClose,
	onCreateNote,
	onUpdateNote,
	onDeleteNote,
}: Readonly<NodeDetailsPanelProps>) {
	const [newNote, setNewNote] = useState("");
	const [isAdding, setIsAdding] = useState(false);
	const [showNoteForm, setShowNoteForm] = useState(false);
	const canAnnotate = app.can_annotate && !app.unknown;

	const handleAdd = useCallback(async () => {
		if (!onCreateNote || !newNote.trim()) return;
		try {
			setIsAdding(true);
			await onCreateNote(app.id, newNote.trim());
			setNewNote("");
			setShowNoteForm(false);
			toast.success("Note added");
		} catch (error) {
			console.error(error);
			toast.error(
				error instanceof Error && error.message
					? error.message
					: "Failed to add note",
			);
		} finally {
			setIsAdding(false);
		}
	}, [app.id, newNote, onCreateNote]);

	const handleUpdate = useCallback(
		async (noteId: string, content: string) => {
			if (!onUpdateNote) return;
			await onUpdateNote(app.id, noteId, content);
		},
		[app.id, onUpdateNote],
	);

	const handleDelete = useCallback(
		async (noteId: string) => {
			if (!onDeleteNote) return;
			await onDeleteNote(app.id, noteId);
		},
		[app.id, onDeleteNote],
	);

	const showBanner = !app.unknown && Boolean(app.banner);

	return (
		<Card className="relative flex h-full w-full flex-col overflow-hidden rounded-none border-0 bg-card shadow-none">
			<Button
				variant="ghost"
				size="icon"
				className="absolute right-2 top-2 z-20 h-7 w-7 bg-background/60 backdrop-blur-sm hover:bg-background/80"
				onClick={onClose}
				aria-label="Close details"
			>
				<X className="h-4 w-4" />
			</Button>
			{showBanner && (
				<div className="relative h-20 w-full shrink-0 overflow-hidden bg-muted">
					<img
						src={app.banner ?? undefined}
						alt=""
						className="h-full w-full object-cover"
					/>
					<div className="absolute inset-0 bg-linear-to-t from-card to-transparent" />
				</div>
			)}
			<div
				className={cn(
					"relative shrink-0 border-b p-4",
					!showBanner &&
						"bg-linear-to-br from-primary/5 via-transparent to-transparent",
				)}
			>
				<div className="flex items-center gap-3 pr-7">
					<Avatar
						className={cn(
							"h-12 w-12 rounded-lg border bg-card",
							showBanner && "-mt-10 h-14 w-14 shadow-sm ring-4 ring-card",
						)}
					>
						<AvatarImage
							src={app.icon ?? undefined}
							alt={`${appLabel(app)} icon`}
						/>
						<AvatarFallback className="rounded-lg bg-primary/10">
							<Blocks className="h-5 w-5 text-primary" />
						</AvatarFallback>
					</Avatar>
					<div className="min-w-0 flex-1">
						<h3
							className={cn(
								"truncate font-semibold",
								app.unknown && "italic text-muted-foreground",
							)}
						>
							{appLabel(app)}
						</h3>
						<div className="mt-1 flex flex-wrap items-center gap-1">
							{app.is_current && (
								<Badge className="h-4 gap-1 px-1.5 text-[10px]">
									<Sparkles className="h-2.5 w-2.5" />
									This app
								</Badge>
							)}
							{app.unknown && (
								<Badge
									variant="outline"
									className="h-4 gap-1 px-1.5 text-[10px] text-muted-foreground"
								>
									<Lock className="h-2.5 w-2.5" />
									No access
								</Badge>
							)}
							{!app.unknown && app.category && (
								<Badge
									variant="secondary"
									className="h-4 px-1.5 text-[10px] font-normal"
								>
									{prettifyCategory(app.category)}
								</Badge>
							)}
						</div>
					</div>
				</div>
				{!app.unknown && app.description && (
					<p className="mt-3 line-clamp-3 text-xs leading-relaxed text-muted-foreground">
						{app.description}
					</p>
				)}
				{!app.unknown && (app.website || app.docs_url) && (
					<div className="mt-2 flex flex-wrap gap-2">
						{app.website && (
							<a
								href={app.website}
								target="_blank"
								rel="noreferrer"
								className="flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground"
							>
								<ExternalLink className="h-3 w-3" />
								Website
							</a>
						)}
						{app.docs_url && (
							<a
								href={app.docs_url}
								target="_blank"
								rel="noreferrer"
								className="flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground"
							>
								<BookOpen className="h-3 w-3" />
								Docs
							</a>
						)}
					</div>
				)}
			</div>
			<ScrollArea className="min-h-0 flex-1">
				<div className="space-y-5 p-4">
					{app.unknown ? (
						<NoAccessPlaceholder />
					) : (
						<>
							{app.tags.length > 0 && (
								<div className="flex flex-wrap gap-1">
									{app.tags.slice(0, 12).map((tag) => (
										<Badge
											key={tag}
											variant="secondary"
											className="text-[10px]"
										>
											{tag}
										</Badge>
									))}
								</div>
							)}
							{app.content && <ContentStats content={app.content} />}
							<AppTables appId={app.id} />
							<PanelSection
								icon={StickyNote}
								title="Process Notes"
								count={app.notes.length}
							>
								{app.notes.length === 0 && !showNoteForm && (
									<p className="text-xs text-muted-foreground">
										No process notes yet.
									</p>
								)}
								{app.notes.map((note) => (
									<NoteItem
										key={note.id}
										note={note}
										canAnnotate={canAnnotate}
										onUpdate={onUpdateNote ? handleUpdate : undefined}
										onDelete={onDeleteNote ? handleDelete : undefined}
									/>
								))}
								{canAnnotate &&
									onCreateNote &&
									(showNoteForm ? (
										<div className="space-y-2 pt-1">
											<Textarea
												placeholder="Describe this app's role in the process..."
												value={newNote}
												maxLength={MAX_NOTE_LENGTH}
												onChange={(e) => setNewNote(e.target.value)}
												className="min-h-17.5 resize-none text-sm"
											/>
											<div className="flex justify-end gap-2">
												<Button
													variant="outline"
													size="sm"
													onClick={() => {
														setShowNoteForm(false);
														setNewNote("");
													}}
												>
													Cancel
												</Button>
												<Button
													size="sm"
													onClick={handleAdd}
													disabled={isAdding || !newNote.trim()}
												>
													{isAdding ? "Adding..." : "Add Note"}
												</Button>
											</div>
										</div>
									) : (
										<Button
											variant="outline"
											size="sm"
											className="w-full"
											onClick={() => setShowNoteForm(true)}
										>
											<Plus className="mr-1.5 h-3.5 w-3.5" />
											Add note
										</Button>
									))}
							</PanelSection>
						</>
					)}
				</div>
			</ScrollArea>
		</Card>
	);
}

function CapabilityRow({
	icon: Icon,
	label,
	access,
}: Readonly<{ icon: LucideIcon; label: string; access?: AccessLevel }>) {
	return (
		<div className="flex items-center justify-between rounded-md border bg-muted/30 px-2.5 py-1.5">
			<span className="flex items-center gap-2 text-xs">
				<Icon className="h-3.5 w-3.5 text-muted-foreground" />
				{label}
			</span>
			{access ? (
				<span
					className={cn(
						"flex items-center gap-1 text-[11px]",
						access === "readwrite"
							? "font-medium text-foreground"
							: "text-muted-foreground",
					)}
				>
					<AccessIcon access={access} className="h-3 w-3" />
					{ACCESS_LABEL[access]}
				</span>
			) : (
				<span className="text-[11px] text-muted-foreground">Execute</span>
			)}
		</div>
	);
}

interface EdgeDetailsPanelProps {
	edge: ConnectionEdgeData;
	nodesById: Map<string, IProcessGraphNode>;
	onClose: () => void;
}

function EdgeDetailsPanel({
	edge,
	nodesById,
	onClose,
}: Readonly<EdgeDetailsPanelProps>) {
	const source = nodesById.get(edge.source);
	const target = nodesById.get(edge.target);
	const sourceName = source ? appLabel(source) : "Unknown App";
	const targetName = target ? appLabel(target) : "Unknown App";
	const caps = edge.capabilities;
	const permissions = grantedPermissionLabels(edge.permissions);
	const statusLabel = edge.pending
		? "Pending"
		: edge.roleName
			? "Active"
			: "Observed";

	return (
		<Card className="relative flex h-full w-full flex-col overflow-hidden rounded-none border-0 bg-card shadow-none">
			<Button
				variant="ghost"
				size="icon"
				className="absolute right-2 top-2 z-20 h-7 w-7"
				onClick={onClose}
				aria-label="Close details"
			>
				<X className="h-4 w-4" />
			</Button>
			<div className="shrink-0 border-b p-4">
				<h3 className="pr-7 text-sm font-semibold">Connection</h3>
				<div className="mt-2 flex items-center gap-1.5 text-sm">
					<span className="min-w-0 flex-1 truncate font-medium">
						{sourceName}
					</span>
					<ArrowRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
					<span className="min-w-0 flex-1 truncate text-right font-medium">
						{targetName}
					</span>
				</div>
				<div className="mt-2 flex flex-wrap items-center gap-1.5">
					{edge.roleName && (
						<Badge variant="secondary" className="gap-1 text-[10px]">
							<Shield className="h-2.5 w-2.5" />
							{edge.roleName}
						</Badge>
					)}
					<Badge
						variant={edge.pending ? "outline" : "secondary"}
						className="text-[10px]"
					>
						{statusLabel}
					</Badge>
					{edge.runCount !== undefined && (
						<Badge variant="secondary" className="text-[10px]">
							{edge.runCount} {edge.runCount === 1 ? "run" : "runs"}
						</Badge>
					)}
				</div>
			</div>
			<ScrollArea className="min-h-0 flex-1">
				<div className="space-y-5 p-4">
					{hasConnectionCapabilities(caps) ? (
						<PanelSection icon={Shield} title="Access granted">
							<div className="space-y-1.5">
								{caps.events && <CapabilityRow icon={Zap} label="Events" />}
								{caps.database && (
									<CapabilityRow
										icon={Database}
										label="Database"
										access={caps.database}
									/>
								)}
								{caps.files && (
									<CapabilityRow
										icon={FileText}
										label="Files"
										access={caps.files}
									/>
								)}
							</div>
						</PanelSection>
					) : (
						<p className="text-xs text-muted-foreground">
							{edge.observed
								? "Observed call chain — no static connection details to show."
								: "This connection grants no event or data access."}
						</p>
					)}
					{permissions.length > 0 && (
						<PanelSection
							icon={Lock}
							title="Role permissions"
							count={permissions.length}
						>
							<div className="flex flex-wrap gap-1">
								{permissions.map((permission) => (
									<Badge
										key={permission}
										variant="outline"
										className="text-[10px] font-normal"
									>
										{permission}
									</Badge>
								))}
							</div>
						</PanelSection>
					)}
				</div>
			</ScrollArea>
		</Card>
	);
}

function GraphLegend() {
	return (
		<div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
			<span className="flex items-center gap-1.5">
				<span className="h-0.5 w-5 rounded bg-primary" />
				Observed
			</span>
			<span className="flex items-center gap-1.5">
				<span className="h-0.5 w-5 rounded bg-muted-foreground" />
				Granted
			</span>
			<span className="flex items-center gap-1.5">
				<span className="w-5 border-t-2 border-dashed border-muted-foreground" />
				Pending
			</span>
			<span className="h-3 w-px bg-border" aria-hidden />
			<span className="flex items-center gap-1">
				<Zap className="h-3 w-3" />
				Events
			</span>
			<span className="flex items-center gap-1">
				<Eye className="h-3 w-3" />
				Read
			</span>
			<span className="flex items-center gap-1 font-medium text-foreground">
				<Pencil className="h-3 w-3" />
				Read/Write
			</span>
		</div>
	);
}

interface ObservedFlowsProps {
	flows: IProcessFlow[];
	nodesById: Map<string, IProcessGraphNode>;
	onHoverPath?: (path: string[] | null) => void;
}

function ObservedFlows({
	flows,
	nodesById,
	onHoverPath,
}: Readonly<ObservedFlowsProps>) {
	const sorted = useMemo(
		() => [...flows].sort((a, b) => b.run_count - a.run_count),
		[flows],
	);

	return (
		<Card>
			<CardHeader className="pb-3">
				<CardTitle className="flex items-center gap-2 text-base">
					<GitBranch className="h-4 w-4" />
					Observed Chains
				</CardTitle>
				<CardDescription>
					Event executions observed across connected apps in the selected time
					window. Data access (database and files) is governed per connection —
					see the capabilities on each graph edge.
				</CardDescription>
			</CardHeader>
			<CardContent>
				{sorted.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						No runs observed in this time window.
					</p>
				) : (
					<div className="space-y-2">
						{sorted.map((flow) => (
							<div
								key={`${flow.path.join("→")}-${flow.event_name ?? ""}`}
								className="flex flex-wrap items-center justify-between gap-2 rounded-md border p-2.5 transition-colors hover:bg-muted/40"
								onMouseEnter={() => onHoverPath?.(flow.path)}
								onMouseLeave={() => onHoverPath?.(null)}
							>
								<div className="flex min-w-0 flex-col gap-1">
									<div className="flex min-w-0 flex-wrap items-center gap-1 text-sm">
										{flow.path.map((appId, index) => {
											const node = nodesById.get(appId);
											const label = node ? appLabel(node) : "Unknown App";
											const isUnknown = !node || node.unknown;
											return (
												<span
													key={`${appId}-${String(index)}`}
													className="flex items-center gap-1"
												>
													{index > 0 && (
														<ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
													)}
													<span
														className={cn(
															"font-medium",
															isUnknown && "italic text-muted-foreground",
														)}
													>
														{label}
													</span>
												</span>
											);
										})}
									</div>
									{flow.event_name && (
										<span className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
											<Zap className="h-3 w-3 shrink-0" />
											<span className="truncate">{flow.event_name}</span>
											{flow.event_type && (
												<span className="shrink-0 opacity-70">
													· {flow.event_type}
												</span>
											)}
										</span>
									)}
								</div>
								<div className="flex shrink-0 items-center gap-2 text-xs text-muted-foreground">
									<Badge variant="secondary">
										{flow.run_count} {flow.run_count === 1 ? "run" : "runs"}
									</Badge>
									{flow.failed_count > 0 && (
										<Badge variant="destructive">
											{flow.failed_count} failed
										</Badge>
									)}
									{flow.avg_duration_ms != null && (
										<span className="flex items-center gap-1">
											<Clock className="h-3 w-3" />
											{formatDuration(flow.avg_duration_ms)}
										</span>
									)}
									<span>
										{formatDistanceToNow(new Date(flow.last_run_at * 1000), {
											addSuffix: true,
										})}
									</span>
								</div>
							</div>
						))}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

/// Status is conveyed by icon + label (never color alone); the only reserved
/// color is destructive for failures.
const STATUS_META: Record<string, { icon: LucideIcon; className: string }> = {
	Failed: { icon: XCircle, className: "text-destructive" },
	Running: { icon: PlayCircle, className: "text-foreground" },
	Completed: { icon: CheckCircle2, className: "text-muted-foreground" },
};

function CaseStatus({ status }: Readonly<{ status: string }>) {
	const meta = STATUS_META[status] ?? STATUS_META.Completed;
	const Icon = meta.icon;
	return (
		<span
			className={cn(
				"flex shrink-0 items-center gap-1 text-[11px] font-medium",
				meta.className,
			)}
		>
			<Icon className="h-3.5 w-3.5" />
			{status}
		</span>
	);
}

function StatTile({
	icon: Icon,
	label,
	value,
	sub,
	subTone,
}: Readonly<{
	icon: LucideIcon;
	label: string;
	value: string;
	sub?: string;
	subTone?: "destructive" | "muted";
}>) {
	return (
		<div className="rounded-lg border bg-card px-3.5 py-3">
			<div className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
				<Icon className="h-3.5 w-3.5" />
				{label}
			</div>
			<p className="mt-1.5 text-xl font-semibold leading-none tabular-nums">
				{value}
			</p>
			<p
				className={cn(
					"mt-1 min-h-3.5 text-[11px] leading-none",
					subTone === "destructive"
						? "text-destructive"
						: "text-muted-foreground",
				)}
			>
				{sub ?? ""}
			</p>
		</div>
	);
}

/** Summary-before-detail: the window's health at a glance. */
function ProcessStats({
	data,
	cases,
}: Readonly<{ data?: IProcessGraphResponse; cases?: IProcessCase[] }>) {
	const stats = useMemo(() => {
		if (!data) return null;
		const connectedApps = data.nodes.filter((node) => !node.is_current).length;
		const runs = data.flows.reduce((sum, flow) => sum + flow.run_count, 0);
		const failedRuns = data.flows.reduce(
			(sum, flow) => sum + flow.failed_count,
			0,
		);
		const caseList = cases ?? [];
		const failedCases = caseList.filter(
			(processCase) => processCase.status === "Failed",
		).length;
		const durations = caseList
			.map((processCase) => processCase.duration_ms)
			.filter(
				(duration): duration is number => duration != null && duration > 0,
			);
		const avgDuration =
			durations.length > 0
				? durations.reduce((sum, duration) => sum + duration, 0) /
					durations.length
				: null;
		const successRate =
			runs > 0 ? Math.round(((runs - failedRuns) / runs) * 100) : null;
		return {
			connectedApps,
			runs,
			failedRuns,
			caseCount: caseList.length,
			failedCases,
			avgDuration,
			successRate,
		};
	}, [data, cases]);

	if (!stats) return null;

	return (
		<div className="grid grid-cols-2 gap-2 sm:grid-cols-3 xl:grid-cols-5">
			<StatTile
				icon={Blocks}
				label="Connected apps"
				value={String(stats.connectedApps)}
			/>
			<StatTile
				icon={Zap}
				label="Observed runs"
				value={String(stats.runs)}
				sub={stats.failedRuns > 0 ? `${stats.failedRuns} failed` : undefined}
				subTone="destructive"
			/>
			<StatTile
				icon={CheckCircle2}
				label="Success rate"
				value={stats.successRate != null ? `${stats.successRate}%` : "—"}
			/>
			<StatTile
				icon={Workflow}
				label="Process cases"
				value={String(stats.caseCount)}
				sub={stats.failedCases > 0 ? `${stats.failedCases} failed` : undefined}
				subTone="destructive"
			/>
			<StatTile
				icon={Clock}
				label="Avg case time"
				value={
					stats.avgDuration != null ? formatDuration(stats.avgDuration) : "—"
				}
			/>
		</div>
	);
}

const CASE_FILTERS = ["all", "Failed", "Running", "Completed"] as const;
type CaseFilter = (typeof CASE_FILTERS)[number];

interface ProcessCasesCardProps {
	cases: IProcessCase[];
	nodesById: Map<string, IProcessGraphNode>;
	/** Hovering a case dims the canvas to its app path. */
	onHoverPath?: (path: string[] | null) => void;
}

function ProcessCasesCard({
	cases,
	nodesById,
	onHoverPath,
}: Readonly<ProcessCasesCardProps>) {
	const [statusFilter, setStatusFilter] = useState<CaseFilter>("all");
	const [search, setSearch] = useState("");

	const nameOf = useCallback(
		(appId: string) => {
			const node = nodesById.get(appId);
			if (node) return appLabel(node);
			return appId.startsWith("unknown::") ? "Unknown App" : appId;
		},
		[nodesById],
	);

	const counts = useMemo(() => {
		const byStatus: Record<string, number> = {};
		for (const processCase of cases) {
			byStatus[processCase.status] = (byStatus[processCase.status] ?? 0) + 1;
		}
		return byStatus;
	}, [cases]);

	const visible = useMemo(() => {
		const query = search.trim().toLowerCase();
		return cases
			.filter(
				(processCase) =>
					statusFilter === "all" || processCase.status === statusFilter,
			)
			.filter((processCase) => {
				if (!query) return true;
				const haystack = [
					processCase.case_id,
					processCase.root_event_name ?? "",
					...processCase.apps.map(nameOf),
					...Object.entries(processCase.correlation_keys ?? {}).flat(),
				]
					.join(" ")
					.toLowerCase();
				return haystack.includes(query);
			})
			.sort((a, b) => b.last_activity_at - a.last_activity_at);
	}, [cases, statusFilter, search, nameOf]);

	return (
		<Card>
			<CardHeader className="pb-3">
				<CardTitle className="flex items-center gap-2 text-base">
					<Workflow className="h-4 w-4" />
					Process Cases
				</CardTitle>
				<CardDescription>
					End-to-end cases this app started, reconstructed across apps and
					events from the run correlation spine. Hover a case to trace its path
					on the graph.
				</CardDescription>
				{cases.length > 0 && (
					<div className="flex flex-wrap items-center gap-2 pt-2">
						<div className="flex items-center gap-1">
							{CASE_FILTERS.map((filter) => (
								<Button
									key={filter}
									size="sm"
									variant={statusFilter === filter ? "secondary" : "ghost"}
									className="h-7 gap-1.5 px-2.5 text-xs"
									onClick={() => setStatusFilter(filter)}
								>
									{filter === "all" ? "All" : filter}
									<span className="tabular-nums text-muted-foreground">
										{filter === "all" ? cases.length : (counts[filter] ?? 0)}
									</span>
								</Button>
							))}
						</div>
						<div className="relative ml-auto">
							<Search className="absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
							<Input
								value={search}
								onChange={(event) => setSearch(event.target.value)}
								placeholder="Filter by key, event, or app"
								className="h-8 w-60 pl-7 text-xs"
							/>
						</div>
					</div>
				)}
			</CardHeader>
			<CardContent>
				{cases.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						No process cases in this time window. Cases appear when this
						app&apos;s events run — pass correlation keys at invoke time to
						group them by business object.
					</p>
				) : visible.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						No cases match the current filter.
					</p>
				) : (
					<div className="space-y-2">
						{visible.map((processCase) => {
							const keys = processCase.correlation_keys
								? Object.entries(processCase.correlation_keys)
								: [];
							return (
								<div
									key={processCase.case_id}
									title={`Case ${processCase.case_id}`}
									className="space-y-2 rounded-md border p-3 transition-colors hover:bg-muted/40"
									onMouseEnter={() => onHoverPath?.(processCase.apps)}
									onMouseLeave={() => onHoverPath?.(null)}
								>
									<div className="flex items-center justify-between gap-2">
										<div className="flex min-w-0 items-center gap-2">
											<CaseStatus status={processCase.status} />
											<span className="truncate text-sm font-medium">
												{nameOf(processCase.root_app_id)}
											</span>
											{processCase.root_event_name && (
												<span className="truncate text-xs text-muted-foreground">
													· {processCase.root_event_name}
												</span>
											)}
										</div>
										<span className="shrink-0 text-xs text-muted-foreground">
											{formatDistanceToNow(
												new Date(processCase.last_activity_at * 1000),
												{ addSuffix: true },
											)}
										</span>
									</div>

									{processCase.apps.length > 1 && (
										<div className="flex min-w-0 flex-wrap items-center gap-1 text-xs text-muted-foreground">
											{processCase.apps.map((appId, index) => (
												<span
													key={`${processCase.case_id}-${appId}`}
													className="flex items-center gap-1"
												>
													{index > 0 && (
														<ArrowRight className="h-3 w-3 shrink-0" />
													)}
													<span className="truncate">{nameOf(appId)}</span>
												</span>
											))}
										</div>
									)}

									{keys.length > 0 && (
										<div className="flex flex-wrap gap-1">
											{keys.map(([key, value]) => (
												<Badge
													key={key}
													variant="secondary"
													className="gap-1 text-[10px] font-normal"
												>
													<span className="text-muted-foreground">{key}</span>
													{String(value)}
												</Badge>
											))}
										</div>
									)}

									<div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
										<Badge variant="secondary">
											{processCase.run_count}{" "}
											{processCase.run_count === 1 ? "run" : "runs"}
										</Badge>
										{processCase.failed_count > 0 && (
											<Badge variant="destructive">
												{processCase.failed_count} failed
											</Badge>
										)}
										{processCase.duration_ms != null && (
											<span className="flex items-center gap-1">
												<Clock className="h-3 w-3" />
												{formatDuration(processCase.duration_ms)}
											</span>
										)}
									</div>
								</div>
							);
						})}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

export function ProcessGraph({
	data,
	cases,
	casesLoading,
	casesError,
	isLoading,
	days,
	onDaysChange,
	onRefresh,
	onCreateNote,
	onUpdateNote,
	onDeleteNote,
}: Readonly<ProcessGraphProps>) {
	const { resolvedTheme } = useTheme();
	const [selection, setSelection] = useState<
		{ kind: "node"; id: string } | { kind: "edge"; id: string } | null
	>(null);
	const [hoveredPath, setHoveredPath] = useState<string[] | null>(null);
	const [hoveredNodeId, setHoveredNodeId] = useState<string | null>(null);
	const [isExpanded, setIsExpanded] = useState(false);
	const [bottomTab, setBottomTab] = useState<"cases" | "chains">("cases");
	const fitViewRef = useRef<(() => void) | null>(null);

	const colorMode = useMemo(
		() => (resolvedTheme === "dark" ? "dark" : "light"),
		[resolvedTheme],
	);

	const nodesById = useMemo(() => {
		const map = new Map<string, IProcessGraphNode>();
		for (const node of data?.nodes ?? []) map.set(node.id, node);
		return map;
	}, [data]);

	const { baseNodes, baseEdges } = useMemo(() => {
		if (!data) return { baseNodes: [], baseEdges: [] };

		const pairs: [string, string][] = data.edges.map((edge) => [
			edge.source,
			edge.target,
		]);
		for (const flow of data.flows) {
			for (let i = 0; i < flow.path.length - 1; i++) {
				pairs.push([flow.path[i], flow.path[i + 1]]);
			}
		}
		const positions = computeLayout(data.nodes, pairs);

		const baseNodes: ProcessGraphFlowNode[] = data.nodes.map((app) => ({
			id: app.id,
			type: "processApp",
			position: positions.get(app.id) ?? { x: 0, y: 0 },
			data: { app },
		}));

		return { baseNodes, baseEdges: buildEdges(data) };
	}, [data]);

	// Cross-highlight: hovering an observed chain / case dims everything off
	// its path; hovering a node dims everything but the node's neighborhood.
	const highlight = useMemo(() => {
		if (hoveredPath && hoveredPath.length > 0) {
			const nodes = new Set(hoveredPath);
			const edges = new Set<string>();
			for (let i = 0; i < hoveredPath.length - 1; i++) {
				edges.add(`edge-${hoveredPath[i]}-${hoveredPath[i + 1]}`);
			}
			return { nodes, edges };
		}
		if (hoveredNodeId) {
			const nodes = new Set([hoveredNodeId]);
			const edges = new Set<string>();
			for (const edge of baseEdges) {
				const edgeData = edge.data as ConnectionEdgeData;
				if (
					edgeData.source === hoveredNodeId ||
					edgeData.target === hoveredNodeId
				) {
					edges.add(edge.id);
					nodes.add(edgeData.source);
					nodes.add(edgeData.target);
				}
			}
			return { nodes, edges };
		}
		return null;
	}, [hoveredPath, hoveredNodeId, baseEdges]);

	const flowNodes = useMemo(
		() =>
			baseNodes.map((node) => ({
				...node,
				data: {
					...node.data,
					dimmed: highlight ? !highlight.nodes.has(node.id) : false,
				},
			})),
		[baseNodes, highlight],
	);

	const flowEdges = useMemo(
		() =>
			baseEdges.map((edge) => ({
				...edge,
				data: {
					...(edge.data as ConnectionEdgeData),
					dimmed: highlight ? !highlight.edges.has(edge.id) : false,
				},
			})),
		[baseEdges, highlight],
	);

	const selectedApp =
		selection?.kind === "node" ? nodesById.get(selection.id) : undefined;
	const selectedEdge =
		selection?.kind === "edge"
			? (baseEdges.find((edge) => edge.id === selection.id)?.data as
					| ConnectionEdgeData
					| undefined)
			: undefined;

	const closePanel = useCallback(() => setSelection(null), []);

	const handleNodeClick = useCallback(
		(_event: React.MouseEvent, node: Node) =>
			setSelection({ kind: "node", id: node.id }),
		[],
	);
	const handleEdgeClick = useCallback(
		(_event: React.MouseEvent, edge: Edge) =>
			setSelection({ kind: "edge", id: edge.id }),
		[],
	);
	const handlePaneClick = useCallback(() => setSelection(null), []);

	const handleDaysChange = useCallback(
		(value: string) => {
			const parsed = Number.parseInt(value, 10);
			if (!Number.isNaN(parsed)) onDaysChange(parsed);
		},
		[onDaysChange],
	);

	// Escape closes the details panel.
	useEffect(() => {
		if (!selection) return;
		const onKey = (event: KeyboardEvent) => {
			if (event.key === "Escape") setSelection(null);
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [selection]);

	// Re-fit when the canvas geometry changes (drawer toggles, expand toggles).
	const panelOpen = selection !== null;
	useEffect(() => {
		// Read both so this re-fits whenever either toggles.
		void panelOpen;
		void isExpanded;
		const timer = setTimeout(() => fitViewRef.current?.(), 60);
		return () => clearTimeout(timer);
	}, [panelOpen, isExpanded]);

	if (isLoading && !data) {
		return (
			<div className="space-y-4">
				<Skeleton className="h-9 w-full" />
				<Skeleton className="h-130 w-full rounded-lg" />
				<Skeleton className="h-32 w-full rounded-lg" />
			</div>
		);
	}

	const empty = !data || data.nodes.length === 0;

	return (
		<div className="space-y-4">
			<div className="flex flex-wrap items-center gap-3">
				<Select value={days.toString()} onValueChange={handleDaysChange}>
					<SelectTrigger className="w-36">
						<SelectValue placeholder="Time window" />
					</SelectTrigger>
					<SelectContent>
						{TIME_WINDOWS.map((window) => (
							<SelectItem key={window} value={window.toString()}>
								Last {window} days
							</SelectItem>
						))}
					</SelectContent>
				</Select>
				<Button variant="outline" size="sm" onClick={onRefresh}>
					<RefreshCw
						className={cn("mr-2 h-4 w-4", isLoading && "animate-spin")}
					/>
					Refresh
				</Button>
				<Button
					variant="outline"
					size="sm"
					onClick={() => setIsExpanded((expanded) => !expanded)}
					aria-label={isExpanded ? "Collapse graph" : "Expand graph"}
				>
					{isExpanded ? (
						<Minimize2 className="h-4 w-4" />
					) : (
						<Maximize2 className="h-4 w-4" />
					)}
				</Button>
				<div className="ml-auto">
					<GraphLegend />
				</div>
			</div>

			<ProcessStats data={data} cases={cases} />

			<div
				className={cn(
					"relative flex w-full overflow-hidden rounded-lg border",
					isExpanded ? "h-[78vh]" : "h-130",
				)}
			>
				<div className="relative min-w-0 flex-1">
					{empty ? (
						<div className="flex h-full items-center justify-center">
							<EmptyState
								icons={[GitBranch]}
								title="No process data"
								description="No connections or observed call chains were found for this time window."
							/>
						</div>
					) : (
						<ReactFlowProvider>
							<ReactFlow
								suppressHydrationWarning
								className="h-full w-full"
								colorMode={colorMode}
								nodes={flowNodes}
								edges={flowEdges}
								nodeTypes={nodeTypes}
								edgeTypes={edgeTypes}
								nodesDraggable
								nodesConnectable={false}
								elementsSelectable
								edgesFocusable={false}
								onInit={(instance) => {
									fitViewRef.current = () =>
										instance.fitView({ padding: 0.2, duration: 200 });
								}}
								onNodeClick={handleNodeClick}
								onEdgeClick={handleEdgeClick}
								onPaneClick={handlePaneClick}
								onNodeMouseEnter={(_event, node) => setHoveredNodeId(node.id)}
								onNodeMouseLeave={() => setHoveredNodeId(null)}
								fitView
								fitViewOptions={{ padding: 0.25 }}
								minZoom={0.2}
								proOptions={{ hideAttribution: true }}
							>
								<Background
									variant={BackgroundVariant.Dots}
									gap={12}
									size={1}
								/>
								<Controls showInteractive={false} />
								{flowNodes.length > 4 && <MiniMap pannable zoomable />}
							</ReactFlow>
						</ReactFlowProvider>
					)}
				</div>
				{selection && (
					<aside
						aria-label="App details"
						className="absolute inset-y-0 right-0 z-10 w-full border-l bg-card md:relative md:inset-auto md:z-auto md:w-80 md:shrink-0"
					>
						{selection.kind === "node" && selectedApp && (
							<NodeDetailsPanel
								app={selectedApp}
								onClose={closePanel}
								onCreateNote={onCreateNote}
								onUpdateNote={onUpdateNote}
								onDeleteNote={onDeleteNote}
							/>
						)}
						{selection.kind === "edge" && selectedEdge && (
							<EdgeDetailsPanel
								edge={selectedEdge}
								nodesById={nodesById}
								onClose={closePanel}
							/>
						)}
					</aside>
				)}
			</div>

			<div className="flex w-fit items-center gap-1 rounded-lg border bg-card p-1">
				<Button
					size="sm"
					variant={bottomTab === "cases" ? "secondary" : "ghost"}
					className="h-7 gap-1.5 px-2.5 text-xs"
					onClick={() => setBottomTab("cases")}
				>
					<Workflow className="h-3.5 w-3.5" />
					Process Cases
					{cases && (
						<span className="tabular-nums text-muted-foreground">
							{cases.length}
						</span>
					)}
				</Button>
				<Button
					size="sm"
					variant={bottomTab === "chains" ? "secondary" : "ghost"}
					className="h-7 gap-1.5 px-2.5 text-xs"
					onClick={() => setBottomTab("chains")}
				>
					<GitBranch className="h-3.5 w-3.5" />
					Observed Chains
					{data && (
						<span className="tabular-nums text-muted-foreground">
							{data.flows.length}
						</span>
					)}
				</Button>
			</div>

			{bottomTab === "cases" ? (
				cases ? (
					<ProcessCasesCard
						cases={cases}
						nodesById={nodesById}
						onHoverPath={setHoveredPath}
					/>
				) : casesError ? (
					<Card>
						<CardContent className="flex items-center gap-3 p-4">
							<XCircle className="h-4 w-4 shrink-0 text-destructive" />
							<div className="text-sm">
								<p className="font-medium">Process cases unavailable</p>
								<p className="text-xs text-muted-foreground">
									The cases endpoint returned an error. Make sure the API is
									running the latest build and the database migration has been
									applied, then refresh.
								</p>
							</div>
						</CardContent>
					</Card>
				) : casesLoading ? (
					<Skeleton className="h-24 w-full rounded-lg" />
				) : (
					<Card>
						<CardContent className="p-4 text-sm text-muted-foreground">
							No process cases loaded yet.
						</CardContent>
					</Card>
				)
			) : (
				<ObservedFlows
					flows={data?.flows ?? []}
					nodesById={nodesById}
					onHoverPath={setHoveredPath}
				/>
			)}
		</div>
	);
}
