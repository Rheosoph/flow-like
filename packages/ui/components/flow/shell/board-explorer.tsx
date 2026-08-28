"use client";

import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import {
	ChevronDownIcon,
	ChevronRightIcon,
	ExternalLinkIcon,
	FileCode2Icon,
	FolderInputIcon,
	LayoutTemplateIcon,
	LockIcon,
	PencilLineIcon,
	PlusIcon,
	Trash2Icon,
	TriangleAlertIcon,
} from "lucide-react";
import { memo, useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks";
import { type PeerUserInfo, colorFromSub } from "../../../hooks/use-peer-users";
import type { IGenericCommand } from "../../../lib";
import {
	FLOWSCRIPT_KEYWORDS,
	type IModuleNameError,
	MAIN_FILE_ID,
	MAIN_FILE_LABEL,
	MODULE_FILE_EXTENSION,
	validateModuleName,
} from "../../../lib/flow-modules";
import { owningModuleId } from "../../../lib/layer-to-function";
import {
	type PresenceMark,
	mergePresenceMarks,
} from "../../../lib/realtime/presence-locations";
import {
	type IBoard,
	type ILayer,
	ILayerType,
} from "../../../lib/schema/flow/board";
import { userInitials } from "../../../lib/user-display";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Avatar, AvatarFallback, AvatarImage } from "../../ui/avatar";
import { Button } from "../../ui/button";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuSub,
	ContextMenuSubContent,
	ContextMenuSubTrigger,
	ContextMenuTrigger,
} from "../../ui/context-menu";
import { Input } from "../../ui/input";
import { useModuleCommands } from "../use-module-commands";

interface ModuleNode {
	layer: ILayer;
	children: ModuleNode[];
}

/** Module layers nest by their nearest *module* ancestor, which is what their path label reflects. */
function buildModuleTree(layers: Record<string, ILayer> | undefined): {
	roots: ModuleNode[];
	all: ILayer[];
} {
	const modules = Object.values(layers ?? {}).filter(
		(layer) => layer.type === ILayerType.Module,
	);
	const nodes = new Map<string, ModuleNode>(
		modules.map((layer) => [layer.id, { layer, children: [] }]),
	);
	const roots: ModuleNode[] = [];

	for (const layer of modules) {
		const parentId = owningModuleId(layers, layer.id);
		const parent = parentId ? nodes.get(parentId) : undefined;
		const node = nodes.get(layer.id);
		if (!node) continue;
		if (parent) parent.children.push(node);
		else roots.push(node);
	}

	const sort = (list: ModuleNode[]) => {
		list.sort((a, b) => a.layer.name.localeCompare(b.layer.name));
		for (const child of list) sort(child.children);
	};
	sort(roots);

	return { roots, all: modules };
}

// Extra props (and ref) must reach the root div so `ContextMenuTrigger asChild`
// can attach its right-click handler.
function TreeRow({
	depth,
	icon,
	label,
	active,
	muted,
	trailing,
	expander,
	onSelect,
	className,
	...rest
}: Readonly<{
	depth: number;
	icon: React.ReactNode;
	label: string;
	active?: boolean;
	muted?: boolean;
	trailing?: React.ReactNode;
	expander?: React.ReactNode;
	onSelect?: () => void;
}> &
	Omit<React.ComponentProps<"div">, "children">) {
	return (
		<div
			{...rest}
			className={cn(
				"group/row flex items-center gap-1 rounded-sm pr-1 text-xs",
				active ? "bg-accent text-accent-foreground" : "hover:bg-accent/60",
				className,
			)}
			style={{ paddingLeft: `${depth * 12 + 4}px` }}
		>
			<span className="flex size-4 shrink-0 items-center justify-center">
				{expander}
			</span>
			<button
				type="button"
				onClick={onSelect}
				className="flex min-w-0 flex-1 items-center gap-1.5 py-1 text-left"
			>
				<span
					className={cn(
						"shrink-0 [&>svg]:size-3.5",
						active
							? "text-accent-foreground"
							: muted
								? "text-muted-foreground"
								: "text-primary",
					)}
				>
					{icon}
				</span>
				<span className="truncate font-mono">{label}</span>
			</button>
			{trailing}
		</div>
	);
}

const MAX_PRESENCE_DOTS = 3;
const NO_MARKS: PresenceMark[] = [];

/**
 * Who is at a place — a file, a layer, a node — as a facepile small enough to
 * sit in a tree row. Shared with the inspector so the same person looks the
 * same in both rails.
 */
export const PresenceDots = memo(function PresenceDots({
	marks,
	peerUsers,
	className,
}: Readonly<{
	marks: readonly PresenceMark[];
	peerUsers?: Map<string, PeerUserInfo>;
	className?: string;
}>) {
	const { t } = useTranslation("flow");
	if (marks.length === 0) return null;
	const shown = marks.slice(0, MAX_PRESENCE_DOTS);
	const overflow = marks.length - shown.length;
	const names = marks
		.map((mark) =>
			mark.self
				? t("you", "You")
				: (peerUsers?.get(mark.sub)?.name ?? mark.sub.slice(-8)),
		)
		.join(", ");
	return (
		<span
			className={cn("flex shrink-0 items-center -space-x-1", className)}
			aria-label={t("presenceOpenBy", {
				defaultValue: "Open by {{names}}",
				names,
			})}
		>
			{shown.map((mark) => {
				const info = peerUsers?.get(mark.sub);
				const color = info?.color ?? colorFromSub(mark.sub);
				const displayName = info?.name ?? mark.sub.slice(-8);
				const label = mark.self ? t("you", "You") : displayName;
				return (
					<Avatar
						key={mark.sub}
						className="size-4 rounded-full ring-1 ring-background"
						style={{ boxShadow: `0 0 0 1px ${color}` }}
						title={mark.sessions > 1 ? `${label} ×${mark.sessions}` : label}
						aria-hidden="true"
					>
						{info?.avatarUrl && (
							<AvatarImage
								src={info.avatarUrl}
								alt=""
								className="object-cover"
							/>
						)}
						<AvatarFallback
							className="rounded-full text-[8px] font-semibold leading-none text-white"
							style={{ background: color }}
						>
							{userInitials(displayName).charAt(0)}
						</AvatarFallback>
					</Avatar>
				);
			})}
			{overflow > 0 && (
				<span
					className="flex size-3.5 items-center justify-center rounded-full bg-muted text-[7px] font-semibold leading-none text-muted-foreground ring-1 ring-background"
					title={marks
						.slice(MAX_PRESENCE_DOTS)
						.map((mark) =>
							mark.self
								? t("you", "You")
								: (peerUsers?.get(mark.sub)?.name ?? mark.sub.slice(-8)),
						)
						.join(", ")}
				>
					+{overflow}
				</span>
			)}
		</span>
	);
});

/** A row's trailing slot with presence in front of whatever control it already had. */
function withPresence(
	dots: React.ReactNode,
	control: React.ReactNode,
): React.ReactNode {
	if (!dots) return control;
	if (!control) return dots;
	return (
		<span className="flex shrink-0 items-center gap-1">
			{dots}
			{control}
		</span>
	);
}

function SectionHeader({
	label,
	action,
}: Readonly<{ label: string; action?: React.ReactNode }>) {
	return (
		<div className="flex items-center gap-1 px-1 pb-0.5 pt-2">
			<h3 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
				{label}
			</h3>
			<span className="flex-1" />
			{action}
		</div>
	);
}

/**
 * The board's two trees.
 *
 * `Flow` is the files the canvas and FlowScript open — `main.flow` plus one node
 * per module layer, nested by module, which is what makes a module a folder.
 * `UI` is the pages the board actually has, each a jump into the page builder.
 * Everything the tab strip can do to a file it can do here too, and here it can
 * also be reparented.
 */
export function BoardExplorer({
	appId,
	boardId,
	board,
	currentFileId,
	onSelectFile,
	onOpenPage,
	executeCommand,
	readOnly,
	reservedRoots = FLOWSCRIPT_KEYWORDS,
	presenceByFile,
	presenceByLayer,
	peerUsers,
}: Readonly<{
	appId: string;
	boardId: string;
	board?: IBoard;
	/** `main` or a module layer id. */
	currentFileId: string;
	onSelectFile: (moduleId: string | null) => void;
	onOpenPage: (pageId: string, boardId: string) => void;
	executeCommand: (
		command: IGenericCommand,
		append: boolean,
	) => Promise<unknown>;
	readOnly: boolean;
	reservedRoots?: readonly string[];
	/** Who has which file open in code, keyed by `main` or a module layer id. */
	presenceByFile?: Map<string, PresenceMark[]>;
	/** Who has which layer open on the canvas, keyed by layer id. */
	presenceByLayer?: Map<string, PresenceMark[]>;
	peerUsers?: Map<string, PeerUserInfo>;
}>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const { createModule, renameModule, moveModule, deleteModule } =
		useModuleCommands(executeCommand);

	const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
	const [renaming, setRenaming] = useState<string | null>(null);
	const [draftParent, setDraftParent] = useState<string | null | undefined>();
	const [draftingPage, setDraftingPage] = useState(false);

	const { roots, all } = useMemo(
		() => buildModuleTree(board?.layers),
		[board?.layers],
	);

	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId, boardId],
		Boolean(appId && boardId),
		[appId, boardId],
	);

	const nameErrorText = useCallback(
		(error: IModuleNameError | null) => {
			if (!error) return null;
			switch (error) {
				case "empty":
					return t("nameIsRequired", "Name is required");
				case "reserved":
					return t("thatNameIsReserved", "That name is reserved");
				case "duplicate":
					return t("thatNameIsAlreadyTaken", "That name is already taken");
				default:
					return t("useALetterOrDigitName", "Use a letter or digit name");
			}
		},
		[t],
	);

	const commitCreate = useCallback(
		async (name: string, parentId: string | null) => {
			setDraftParent(undefined);
			const layer = await createModule(name, parentId);
			onSelectFile(layer.id);
		},
		[createModule, onSelectFile],
	);

	const commitRename = useCallback(
		async (id: string, name: string) => {
			const layer = board?.layers?.[id];
			setRenaming(null);
			if (layer) await renameModule(layer, name);
		},
		[board?.layers, renameModule],
	);

	// A page's route is not its name — the home page lives at "/" — so both are
	// asked for up front. Route editing after the fact needs a full IPage
	// round-trip, so getting it wrong here is expensive to undo.
	const createPage = useCallback(
		async (name: string, route: string) => {
			setDraftingPage(false);
			const trimmed = name.trim();
			if (!trimmed) return;
			try {
				await backend.pageState.createPage(
					appId,
					createId(),
					trimmed,
					route.trim() || "/",
					boardId,
				);
				await pages.refetch();
			} catch (error) {
				console.error("Failed to create page", error);
				toast.error(t("failedToCreatePage", "Failed to create page"));
			}
		},
		[appId, boardId, backend.pageState, pages, t],
	);

	const deletePage = useCallback(
		async (pageId: string) => {
			try {
				await backend.pageState.deletePage(appId, pageId, boardId);
				await pages.refetch();
			} catch (error) {
				console.error("Failed to delete page", error);
				toast.error(t("failedToDeletePage", "Failed to delete page"));
			}
		},
		[appId, boardId, backend.pageState, pages, t],
	);

	const toggle = useCallback((id: string) => {
		setCollapsed((old) => {
			const next = new Set(old);
			if (next.has(id)) next.delete(id);
			else next.add(id);
			return next;
		});
	}, []);

	// A module is both a file and a layer, so its row shows whoever is in
	// either. The parent's arrays are handed through untouched whenever only
	// one side has anyone, which is what keeps the dots' memo intact.
	const marksFor = useCallback(
		(id: string): readonly PresenceMark[] => {
			const inFile = presenceByFile?.get(id);
			const onLayer = presenceByLayer?.get(id);
			if (!inFile) return onLayer ?? NO_MARKS;
			if (!onLayer) return inFile;
			return mergePresenceMarks(inFile, onLayer);
		},
		[presenceByFile, presenceByLayer],
	);

	const presenceDots = (id: string): React.ReactNode => {
		const marks = marksFor(id);
		if (marks.length === 0) return null;
		return <PresenceDots marks={marks} peerUsers={peerUsers} />;
	};

	// Nesting is what makes a module a folder, so the draft must be visible —
	// expand a collapsed parent before opening the name field inside it.
	const startDraftInside = useCallback((parentId: string) => {
		setCollapsed((old) => {
			if (!old.has(parentId)) return old;
			const next = new Set(old);
			next.delete(parentId);
			return next;
		});
		setDraftParent(parentId);
	}, []);

	const renderNameField = (
		initial: string,
		parentId: string | null,
		excludeId: string | undefined,
		depth: number,
		onSubmit: (name: string) => void,
		onCancel: () => void,
	) => (
		<NameField
			key={excludeId ?? `new-${parentId ?? "root"}`}
			initial={initial}
			depth={depth}
			validate={(value) =>
				nameErrorText(
					validateModuleName(
						value,
						board?.layers,
						parentId,
						reservedRoots,
						excludeId,
					),
				)
			}
			onSubmit={onSubmit}
			onCancel={onCancel}
		/>
	);

	const renderModule = (node: ModuleNode, depth: number): React.ReactNode => {
		const { layer, children } = node;
		if (renaming === layer.id) {
			return renderNameField(
				layer.name,
				owningModuleId(board?.layers, layer.id),
				layer.id,
				depth,
				(name) => void commitRename(layer.id, name),
				() => setRenaming(null),
			);
		}

		const isOpen = !collapsed.has(layer.id);
		const isActive = currentFileId === layer.id;
		const row = (
			<TreeRow
				depth={depth}
				icon={<FileCode2Icon />}
				label={`${layer.name}${MODULE_FILE_EXTENSION}`}
				active={isActive}
				onSelect={() => onSelectFile(layer.id)}
				trailing={withPresence(
					presenceDots(layer.id),
					!readOnly ? (
						<button
							type="button"
							title={t("newModuleInside", "New module inside")}
							aria-label={t("newModuleInside", "New module inside")}
							onClick={(event) => {
								event.stopPropagation();
								startDraftInside(layer.id);
							}}
							className={cn(
								"flex size-4 shrink-0 items-center justify-center rounded-sm opacity-0 transition-opacity group-hover/row:opacity-100",
								isActive
									? "text-accent-foreground/70 hover:text-accent-foreground"
									: "text-muted-foreground hover:text-foreground",
							)}
						>
							<PlusIcon className="size-3" />
						</button>
					) : undefined,
				)}
				expander={
					children.length > 0 ? (
						<button
							type="button"
							aria-label={layer.name}
							onClick={() => toggle(layer.id)}
							className={
								isActive ? "text-accent-foreground/70" : "text-muted-foreground"
							}
						>
							{isOpen ? (
								<ChevronDownIcon className="size-3" />
							) : (
								<ChevronRightIcon className="size-3" />
							)}
						</button>
					) : undefined
				}
			/>
		);

		return (
			<div key={layer.id}>
				{readOnly ? (
					row
				) : (
					<ContextMenu>
						<ContextMenuTrigger asChild>{row}</ContextMenuTrigger>
						<ContextMenuContent className="w-56">
							<ContextMenuItem onSelect={() => setRenaming(layer.id)}>
								<PencilLineIcon className="size-3.5" />
								{t("rename", "Rename")}
							</ContextMenuItem>
							<ContextMenuItem onSelect={() => startDraftInside(layer.id)}>
								<PlusIcon className="size-3.5" />
								{t("newModuleInside", "New module inside")}
							</ContextMenuItem>
							<ContextMenuSub>
								<ContextMenuSubTrigger>
									<FolderInputIcon className="size-3.5" />
									{t("moveTo", "Move to")}
								</ContextMenuSubTrigger>
								<ContextMenuSubContent className="w-56">
									<ContextMenuItem
										disabled={owningModuleId(board?.layers, layer.id) === null}
										onSelect={() => void moveModule(layer, null)}
									>
										{MAIN_FILE_LABEL}
									</ContextMenuItem>
									{all
										.filter(
											(candidate) =>
												candidate.id !== layer.id &&
												!isDescendant(board?.layers, candidate.id, layer.id),
										)
										.map((candidate) => (
											<ContextMenuItem
												key={candidate.id}
												onSelect={() => void moveModule(layer, candidate.id)}
											>
												{candidate.name}
											</ContextMenuItem>
										))}
								</ContextMenuSubContent>
							</ContextMenuSub>
							<ContextMenuSeparator />
							<ContextMenuItem
								variant="destructive"
								onSelect={() => void deleteModule(layer, true)}
							>
								<Trash2Icon className="size-3.5" />
								{t("deleteModule", "Delete module")}
							</ContextMenuItem>
						</ContextMenuContent>
					</ContextMenu>
				)}
				{isOpen && children.map((child) => renderModule(child, depth + 1))}
				{draftParent === layer.id &&
					renderNameField(
						"",
						layer.id,
						undefined,
						depth + 1,
						(name) => void commitCreate(name, layer.id),
						() => setDraftParent(undefined),
					)}
			</div>
		);
	};

	return (
		<div className="flex flex-col gap-0.5 p-1">
			<SectionHeader
				label={t("flow", "Flow")}
				action={
					!readOnly && (
						<Button
							size="icon"
							variant="ghost"
							className="size-5 text-muted-foreground"
							title={t("newModule", "New module")}
							aria-label={t("newModule", "New module")}
							onClick={() => setDraftParent(null)}
						>
							<PlusIcon className="size-3.5" />
						</Button>
					)
				}
			/>

			<TreeRow
				depth={0}
				icon={<FileCode2Icon />}
				label={MAIN_FILE_LABEL}
				active={currentFileId === MAIN_FILE_ID}
				onSelect={() => onSelectFile(null)}
				trailing={withPresence(
					presenceDots(MAIN_FILE_ID),
					<LockIcon
						className={cn(
							"size-3 shrink-0",
							currentFileId === MAIN_FILE_ID
								? "text-accent-foreground/60"
								: "text-muted-foreground/50",
						)}
						aria-label={t(
							"theRootFileCannotBeChanged",
							"The root file cannot be changed",
						)}
					/>,
				)}
			/>
			{roots.map((node) => renderModule(node, 0))}
			{draftParent === null &&
				renderNameField(
					"",
					null,
					undefined,
					0,
					(name) => void commitCreate(name, null),
					() => setDraftParent(undefined),
				)}

			<SectionHeader
				label={t("ui", "UI")}
				action={
					// Pages are not versioned with the board, so pinning a board version
					// must not lock them the way it locks the flow files.
					<Button
						size="icon"
						variant="ghost"
						className="size-5 text-muted-foreground"
						title={t("createPage", "Create Page")}
						aria-label={t("createPage", "Create Page")}
						onClick={() => setDraftingPage(true)}
					>
						<PlusIcon className="size-3.5" />
					</Button>
				}
			/>

			{draftingPage && (
				<PageDraftForm
					onSubmit={(name, route) => void createPage(name, route)}
					onCancel={() => setDraftingPage(false)}
				/>
			)}
			{(pages.data?.length ?? 0) === 0 && !draftingPage && (
				<p className="px-2 py-1 text-[11px] text-muted-foreground">
					{t("noPagesYet", "No pages yet")}
				</p>
			)}
			{pages.data?.map((page) => {
				const row = (
					<TreeRow
						key={page.pageId}
						depth={0}
						icon={<LayoutTemplateIcon />}
						label={page.name}
						muted
						onSelect={() => onOpenPage(page.pageId, boardId)}
						trailing={
							page.unavailable ? (
								// The board still lists it, so it is not deleted — its content
								// just is not readable here. Naming that beats a row that opens
								// onto nothing.
								<TriangleAlertIcon
									className="size-3 shrink-0 text-amber-600 dark:text-amber-400"
									aria-label={t(
										"contentUnavailableOnThisDevice",
										"Content unavailable on this device",
									)}
								/>
							) : (
								<ExternalLinkIcon className="size-3 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover/row:opacity-100" />
							)
						}
					/>
				);
				return (
					<ContextMenu key={page.pageId}>
						<ContextMenuTrigger asChild>{row}</ContextMenuTrigger>
						<ContextMenuContent className="w-56">
							<ContextMenuItem
								onSelect={() => onOpenPage(page.pageId, boardId)}
							>
								<ExternalLinkIcon className="size-3.5" />
								{t("openInBuilder", "Open in Builder")}
							</ContextMenuItem>
							<ContextMenuSeparator />
							<ContextMenuItem
								variant="destructive"
								onSelect={() => void deletePage(page.pageId)}
							>
								<Trash2Icon className="size-3.5" />
								{t("deletePage", "Delete Page")}
							</ContextMenuItem>
						</ContextMenuContent>
					</ContextMenu>
				);
			})}
		</div>
	);
}

/** A module may not be moved inside its own subtree. */
function isDescendant(
	layers: Record<string, ILayer> | undefined,
	candidateId: string,
	ancestorId: string,
): boolean {
	let current: string | null = candidateId;
	for (let depth = 0; current && depth < 40; depth += 1) {
		if (current === ancestorId) return true;
		current = owningModuleId(layers, current);
	}
	return false;
}

function NameField({
	initial,
	depth,
	validate,
	onSubmit,
	onCancel,
}: Readonly<{
	initial: string;
	depth: number;
	validate: (value: string) => string | null;
	onSubmit: (name: string) => void;
	onCancel: () => void;
}>) {
	const [value, setValue] = useState(initial);
	const error = value.trim() ? validate(value) : null;
	const canSubmit = Boolean(value.trim()) && !error;

	return (
		<div
			className="flex flex-col gap-0.5 py-0.5"
			style={{ paddingLeft: `${depth * 12 + 24}px` }}
		>
			<Input
				autoFocus
				value={value}
				aria-invalid={Boolean(error)}
				className="h-6 px-1.5 font-mono text-xs"
				onChange={(event) => setValue(event.target.value)}
				onBlur={() => canSubmit && onSubmit(value.trim())}
				onKeyDown={(event) => {
					if (event.key === "Enter" && canSubmit) onSubmit(value.trim());
					if (event.key === "Escape") onCancel();
				}}
			/>
			{error && <span className="text-[10px] text-destructive">{error}</span>}
		</div>
	);
}

/** `Incident desk` -> `/incident-desk`; the default route until one is typed. */
function routeFromName(name: string): string {
	const slug = name
		.trim()
		.toLowerCase()
		.replace(/[^a-z0-9]+/gi, "-")
		.replace(/^-|-$/g, "");
	return `/${slug}`;
}

function PageDraftForm({
	onSubmit,
	onCancel,
}: Readonly<{
	onSubmit: (name: string, route: string) => void;
	onCancel: () => void;
}>) {
	const { t } = useTranslation("flow");
	const [name, setName] = useState("");
	const [route, setRoute] = useState("");
	const [routeEdited, setRouteEdited] = useState(false);

	const effectiveRoute = routeEdited ? route : routeFromName(name);
	const canSubmit = Boolean(name.trim());
	const submit = () => {
		if (canSubmit) onSubmit(name, effectiveRoute);
	};

	return (
		<div className="flex flex-col gap-1 py-1 pl-6 pr-1">
			<Input
				autoFocus
				value={name}
				placeholder={t("pageName", "Page Name")}
				className="h-6 px-1.5 text-xs"
				onChange={(event) => setName(event.target.value)}
				onKeyDown={(event) => {
					if (event.key === "Enter") submit();
					if (event.key === "Escape") onCancel();
				}}
			/>
			<Input
				value={effectiveRoute}
				placeholder="/"
				aria-label={t("route", "Route")}
				className="h-6 px-1.5 font-mono text-[11px]"
				onChange={(event) => {
					setRouteEdited(true);
					setRoute(event.target.value);
				}}
				onKeyDown={(event) => {
					if (event.key === "Enter") submit();
					if (event.key === "Escape") onCancel();
				}}
			/>
			<div className="flex items-center gap-1">
				<Button
					size="sm"
					className="h-6 px-2 text-xs"
					disabled={!canSubmit}
					onClick={submit}
				>
					{t("create", "Create")}
				</Button>
				<Button
					size="sm"
					variant="ghost"
					className="h-6 px-2 text-xs"
					onClick={onCancel}
				>
					{t("cancel", "Cancel")}
				</Button>
			</div>
		</div>
	);
}
