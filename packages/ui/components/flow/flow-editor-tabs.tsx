"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CopyPlusIcon,
	DatabaseIcon,
	FileCode2Icon,
	FileIcon,
	LayoutTemplateIcon,
	PaletteIcon,
	PencilLineIcon,
	PinIcon,
	PinOffIcon,
	PlusIcon,
	SquareDashedBottomCodeIcon,
	Trash2Icon,
	XIcon,
} from "lucide-react";
import type { ComponentProps, ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { IGenericCommand } from "../../lib";
import {
	FLOWSCRIPT_KEYWORDS,
	type IModuleNameError,
	MAIN_FILE_ID,
	MAIN_FILE_LABEL,
	modulePathLabel,
	validateModuleName,
} from "../../lib/flow-modules";
import { loadFlowScriptNamesTable } from "../../lib/flowscript/names";
import type { IBoard } from "../../lib/schema/flow/board";
import { cn } from "../../lib/utils";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { Button } from "../ui/button";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuRadioGroup,
	ContextMenuRadioItem,
	ContextMenuSeparator,
	ContextMenuSub,
	ContextMenuSubContent,
	ContextMenuSubTrigger,
	ContextMenuTrigger,
} from "../ui/context-menu";
import { Input } from "../ui/input";
import { Popover, PopoverAnchor, PopoverContent } from "../ui/popover";
import {
	EDITOR_TAB_COLORS,
	type IEditorDocument,
	type IEditorTab,
	type IEditorTabColor,
	isTabClosable,
} from "./shell/editor-documents";
import { useModuleCommands } from "./use-module-commands";

/** Inline name editor, shared by "new module" and "rename". */
function ModuleNameField({
	initialValue,
	submitLabel,
	validate,
	onSubmit,
	onCancel,
}: Readonly<{
	initialValue: string;
	submitLabel: string;
	validate: (value: string) => string | null;
	onSubmit: (name: string) => void;
	onCancel: () => void;
}>) {
	const { t } = useTranslation("flow");
	const [value, setValue] = useState(initialValue);
	// The catalog's namespace roots are reserved names, and they are only complete once the
	// generated names snapshot is in. Naming a module is the first moment that matters, so the
	// (lazy, ~400 KB) snapshot is pulled here rather than on every board open.
	useEffect(() => {
		void loadFlowScriptNamesTable();
	}, []);
	// An untouched field is not a mistake yet — only the submit button reacts to it.
	const error = useMemo(
		() => (value.trim() ? validate(value) : null),
		[validate, value],
	);
	const canSubmit = Boolean(value.trim()) && !error;

	const submit = useCallback(() => {
		if (!canSubmit) return;
		onSubmit(value.trim());
	}, [canSubmit, onSubmit, value]);

	// The error is portalled: the field can sit inside a scrolling tab lane, which clips
	// anything that hangs below it.
	return (
		<Popover open={Boolean(error)}>
			<PopoverAnchor asChild>
				<div className="flex shrink-0 items-center gap-1">
					<FileCode2Icon className="size-3 shrink-0 text-primary" />
					<Input
						autoFocus
						value={value}
						aria-invalid={Boolean(error)}
						aria-label={t("moduleName", "Module name")}
						placeholder={t("moduleNamePlaceholder", "moduleName")}
						className="h-6 w-36 px-1.5 font-mono text-xs"
						onChange={(event) => setValue(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter") submit();
							if (event.key === "Escape") onCancel();
						}}
					/>
					<Button
						size="sm"
						className="h-6 px-2 text-xs"
						disabled={!canSubmit}
						onClick={submit}
					>
						{submitLabel}
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
			</PopoverAnchor>
			<PopoverContent
				role="alert"
				side="bottom"
				align="start"
				onOpenAutoFocus={(event) => event.preventDefault()}
				onCloseAutoFocus={(event) => event.preventDefault()}
				className="w-auto max-w-xs border-destructive/40 px-2 py-1 text-[10px] text-destructive shadow-md"
			>
				{error}
			</PopoverContent>
		</Popover>
	);
}

const KIND_ICONS: Record<IEditorDocument["kind"], typeof FileCode2Icon> = {
	board: FileCode2Icon,
	page: LayoutTemplateIcon,
	widget: SquareDashedBottomCodeIcon,
	storage: FileIcon,
	table: DatabaseIcon,
	styles: PaletteIcon,
};

const TAB_COLOR_CLASSES: Record<
	IEditorTabColor,
	{ readonly fill: string; readonly text: string }
> = {
	red: { fill: "bg-tab-red", text: "text-tab-red" },
	orange: { fill: "bg-tab-orange", text: "text-tab-orange" },
	yellow: { fill: "bg-tab-yellow", text: "text-tab-yellow" },
	green: { fill: "bg-tab-green", text: "text-tab-green" },
	teal: { fill: "bg-tab-teal", text: "text-tab-teal" },
	blue: { fill: "bg-tab-blue", text: "text-tab-blue" },
	purple: { fill: "bg-tab-purple", text: "text-tab-purple" },
	pink: { fill: "bg-tab-pink", text: "text-tab-pink" },
};

const NO_TAB_COLOR = "none";

function useTabColorLabels(): Record<IEditorTabColor, string> {
	const { t } = useTranslation("flow");
	return useMemo(
		() => ({
			red: t("colorRed", "Red"),
			orange: t("colorOrange", "Orange"),
			yellow: t("colorYellow", "Yellow"),
			green: t("colorGreen", "Green"),
			teal: t("colorTeal", "Teal"),
			blue: t("colorBlue", "Blue"),
			purple: t("colorPurple", "Purple"),
			pink: t("colorPink", "Pink"),
		}),
		[t],
	);
}

const LANE_EDGE_PX = 16;

/**
 * One horizontally scrolling run of tabs. Its scrollbar is hidden, so a vertical wheel
 * scrolls it sideways, the active tab is brought into view when it or the set of tabs
 * changes, and a faded edge says there is more past it.
 */
function EditorTabLane({
	activeKey,
	itemsKey,
	className,
	children,
}: Readonly<{
	activeKey: string | null;
	/** Changes whenever the lane's tabs do; scrolling alone must not re-snap to the active tab. */
	itemsKey: string;
	className?: string;
	children: ReactNode;
}>) {
	const laneRef = useRef<HTMLDivElement>(null);
	const rowRef = useRef<HTMLDivElement>(null);
	const [edges, setEdges] = useState({ start: false, end: false });

	const measure = useCallback(() => {
		const lane = laneRef.current;
		if (!lane) return;
		const start = lane.scrollLeft > 1;
		const end = lane.scrollLeft + lane.clientWidth < lane.scrollWidth - 1;
		setEdges((old) =>
			old.start === start && old.end === end ? old : { start, end },
		);
	}, []);

	useEffect(() => {
		const lane = laneRef.current;
		const row = rowRef.current;
		if (!lane || !row || typeof ResizeObserver === "undefined") return;
		const observer = new ResizeObserver(measure);
		observer.observe(lane);
		observer.observe(row);
		return () => observer.disconnect();
	}, [measure]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: itemsKey stands in for the rendered tabs, whose positions this reads from the DOM.
	useEffect(() => {
		const lane = laneRef.current;
		if (!lane || !activeKey) return;
		const tab = Array.from(
			lane.querySelectorAll<HTMLElement>("[data-tab-key]"),
		).find((element) => element.dataset.tabKey === activeKey);
		if (!tab) return;
		const left = tab.offsetLeft;
		const right = left + tab.offsetWidth;
		if (left < lane.scrollLeft) {
			lane.scrollLeft = Math.max(0, left - LANE_EDGE_PX);
		} else if (right > lane.scrollLeft + lane.clientWidth) {
			lane.scrollLeft = right - lane.clientWidth + LANE_EDGE_PX;
		}
		measure();
	}, [activeKey, itemsKey, measure]);

	const mask =
		edges.start || edges.end
			? `linear-gradient(to right, ${edges.start ? "transparent" : "black"}, black ${LANE_EDGE_PX}px, black calc(100% - ${LANE_EDGE_PX}px), ${edges.end ? "transparent" : "black"})`
			: undefined;

	return (
		<div
			ref={laneRef}
			onScroll={measure}
			onWheel={(event) => {
				if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
				event.currentTarget.scrollLeft += event.deltaY;
			}}
			style={{ maskImage: mask, WebkitMaskImage: mask }}
			className={cn("no-scrollbar overflow-x-auto py-1", className)}
		>
			<div ref={rowRef} className="relative flex w-max items-center gap-1">
				{children}
			</div>
		</div>
	);
}

// Extra props (and ref) must reach the root span so `ContextMenuTrigger asChild`
// can attach its right-click handler.
function EditorTabButton({
	tab,
	label,
	active,
	onSelect,
	onClose,
	onUnpin,
	className,
	...rest
}: Readonly<{
	tab: IEditorTab;
	label: string;
	active: boolean;
	onSelect: () => void;
	/** Absent for the last board tab, which is what the canvas falls back to. */
	onClose?: () => void;
	onUnpin: () => void;
}> &
	Omit<ComponentProps<"span">, "children">) {
	const { t } = useTranslation("flow");
	const Icon = KIND_ICONS[tab.doc.kind];
	const tint = tab.color ? TAB_COLOR_CLASSES[tab.color] : undefined;
	return (
		<span
			{...rest}
			data-tab-key={tab.key}
			className={cn(
				"group/tab relative flex shrink-0 items-center rounded-md border pr-1 transition-colors",
				active
					? "border-border bg-background text-foreground shadow-sm"
					: "border-transparent text-muted-foreground hover:bg-muted/60 hover:text-foreground",
				className,
			)}
		>
			<button
				type="button"
				onClick={onSelect}
				title={label}
				aria-current={active ? "page" : undefined}
				className="flex items-center gap-1.5 py-1 pl-2.5 pr-1 font-mono text-xs"
			>
				<Icon className={cn("size-3 shrink-0", tint?.text)} />
				<span className={cn("truncate", tab.pinned ? "max-w-32" : "max-w-48")}>
					{label}
				</span>
			</button>
			{tab.pinned ? (
				<button
					type="button"
					aria-label={t("unpinTab", "Unpin tab")}
					title={t("unpinTab", "Unpin tab")}
					onClick={(event) => {
						event.stopPropagation();
						onUnpin();
					}}
					className="flex size-4 shrink-0 items-center justify-center rounded-sm text-muted-foreground hover:bg-accent hover:text-foreground"
				>
					<PinIcon className="size-3" />
				</button>
			) : (
				onClose && (
					<button
						type="button"
						aria-label={t("closeFile", "Close file")}
						title={t("closeFile", "Close file")}
						onClick={(event) => {
							event.stopPropagation();
							onClose();
						}}
						className={cn(
							"flex size-4 shrink-0 items-center justify-center rounded-sm text-muted-foreground transition-opacity hover:bg-accent hover:text-foreground",
							active ? "opacity-100" : "opacity-0 group-hover/tab:opacity-100",
						)}
					>
						<XIcon className="size-3" />
					</button>
				)
			)}
			{tint && (
				<span
					aria-hidden
					className={cn(
						"pointer-events-none absolute inset-x-2 bottom-0 h-0.5 rounded-full",
						tint.fill,
					)}
				/>
			)}
		</span>
	);
}

function TabColorMenu({
	color,
	onColor,
}: Readonly<{
	color: IEditorTabColor | undefined;
	onColor: (color: IEditorTabColor | undefined) => void;
}>) {
	const { t } = useTranslation("flow");
	const labels = useTabColorLabels();
	return (
		<ContextMenuSub>
			<ContextMenuSubTrigger className="gap-2">
				<PaletteIcon className="size-3.5 text-muted-foreground" />
				{t("tabColor", "Tab color")}
			</ContextMenuSubTrigger>
			<ContextMenuSubContent className="w-40">
				<ContextMenuRadioGroup
					value={color ?? NO_TAB_COLOR}
					onValueChange={(value) =>
						onColor(EDITOR_TAB_COLORS.find((option) => option === value))
					}
				>
					<ContextMenuRadioItem value={NO_TAB_COLOR}>
						{t("noColor", "No color")}
					</ContextMenuRadioItem>
					{EDITOR_TAB_COLORS.map((option) => (
						<ContextMenuRadioItem key={option} value={option}>
							<span
								aria-hidden
								className={cn(
									"size-2.5 shrink-0 rounded-full",
									TAB_COLOR_CLASSES[option].fill,
								)}
							/>
							{labels[option]}
						</ContextMenuRadioItem>
					))}
				</ContextMenuRadioGroup>
			</ContextMenuSubContent>
		</ContextMenuSub>
	);
}

interface IEditorTabActions {
	select: () => void;
	close: () => void;
	split: () => void;
	pin: (pinned: boolean) => void;
	color: (color: IEditorTabColor | undefined) => void;
	rename: () => void;
	delete: () => void;
	deleteWithContents: () => void;
}

function EditorTab({
	tab,
	label,
	active,
	closable,
	readOnly,
	actions,
}: Readonly<{
	tab: IEditorTab;
	label: string;
	active: boolean;
	closable: boolean;
	readOnly: boolean;
	actions: IEditorTabActions;
}>) {
	const { t } = useTranslation("flow");
	const isBoard = tab.doc.kind === "board";
	const isModule = isBoard && tab.doc.fileId !== MAIN_FILE_ID;
	const pinned = Boolean(tab.pinned);

	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				<EditorTabButton
					tab={tab}
					label={label}
					active={active}
					onSelect={actions.select}
					onClose={closable ? actions.close : undefined}
					onUnpin={() => actions.pin(false)}
				/>
			</ContextMenuTrigger>
			<ContextMenuContent className="w-56">
				{closable && (
					<ContextMenuItem onSelect={actions.close}>
						<XIcon className="size-3.5" />
						{t("closeFile", "Close file")}
					</ContextMenuItem>
				)}
				{isBoard && (
					<ContextMenuItem onSelect={actions.split}>
						<CopyPlusIcon className="size-3.5" />
						{t("openInNewTab", "Open in new tab")}
					</ContextMenuItem>
				)}
				{(closable || isBoard) && <ContextMenuSeparator />}
				<ContextMenuItem onSelect={() => actions.pin(!pinned)}>
					{pinned ? (
						<PinOffIcon className="size-3.5" />
					) : (
						<PinIcon className="size-3.5" />
					)}
					{pinned ? t("unpinTab", "Unpin tab") : t("pinTab", "Pin tab")}
				</ContextMenuItem>
				<TabColorMenu color={tab.color} onColor={actions.color} />
				{isModule && !readOnly && (
					<>
						<ContextMenuSeparator />
						<ContextMenuItem onSelect={actions.rename}>
							<PencilLineIcon className="size-3.5" />
							{t("rename", "Rename")}
						</ContextMenuItem>
						<ContextMenuSeparator />
						<ContextMenuItem onSelect={actions.delete}>
							<Trash2Icon className="size-3.5" />
							{t("deleteModule", "Delete module")}
						</ContextMenuItem>
						<ContextMenuItem
							variant="destructive"
							onSelect={actions.deleteWithContents}
						>
							<Trash2Icon className="size-3.5" />
							{t("deleteWithContents", "Delete with contents")}
						</ContextMenuItem>
					</>
				)}
			</ContextMenuContent>
		</ContextMenu>
	);
}

/**
 * The editor's open documents: `.flow` files, pages, widgets, storage files and tables.
 *
 * A `.flow` tab is a position as much as a file — selecting one makes its layer the current
 * layer, and walking deeper on the canvas moves the tab rather than opening another. That is
 * why the same file can be open twice: two tabs, two layer paths, one canvas.
 */
export function FlowEditorTabs({
	board,
	tabs,
	activeKey,
	activeModuleId,
	resolveLabel,
	onSelect,
	onClose,
	onSplit,
	onPin,
	onColor,
	executeCommand,
	readOnly,
	reservedRoots = FLOWSCRIPT_KEYWORDS,
	trailing,
}: Readonly<{
	board: IBoard;
	tabs: readonly IEditorTab[];
	activeKey: string | null;
	/** The module the canvas is inside, so a new module is created in the right place. */
	activeModuleId: string | null;
	/** Label for anything the strip cannot name from the board alone. */
	resolveLabel: (tab: IEditorTab) => string;
	onSelect: (key: string) => void;
	onClose: (key: string) => void;
	onSplit: (key: string) => void;
	onPin: (key: string, pinned: boolean) => void;
	onColor: (key: string, color: IEditorTabColor | undefined) => void;
	executeCommand: (
		command: IGenericCommand,
		append: boolean,
	) => Promise<unknown>;
	readOnly: boolean;
	/** Names the surrounding FlowScript already owns. Defaults to the keywords. */
	reservedRoots?: readonly string[];
	/** Editor-level controls pinned to the right of the file tabs. */
	trailing?: ReactNode;
}>) {
	const { t } = useTranslation("flow");
	const { createModule, renameModule, deleteModule } =
		useModuleCommands(executeCommand);

	const [drafting, setDrafting] = useState(false);
	const [renamingId, setRenamingId] = useState<string | null>(null);
	const [pendingDelete, setPendingDelete] = useState<{
		id: string;
		label: string;
	} | null>(null);
	const [pendingSelect, setPendingSelect] = useState<string | null>(null);

	// A module created a moment ago is not on the board the parent is rendering yet, and
	// opening a layer it cannot resolve would leave the canvas on a broken path. Waiting for
	// it to arrive means parent and strip agree on what exists.
	useEffect(() => {
		if (!pendingSelect) return;
		if (!board.layers?.[pendingSelect]) return;
		setPendingSelect(null);
		onSelect(pendingSelect);
	}, [board.layers, onSelect, pendingSelect]);

	const nameErrorText = useCallback(
		(error: IModuleNameError | null) => {
			switch (error) {
				case "empty":
					return t("moduleNameCannotBeEmpty", "Module name cannot be empty");
				case "invalid_identifier":
					return t(
						"useLettersOrDigitsAModuleNameBecomesACodeIdentifier",
						"Use letters or digits — a module name becomes a code identifier",
					);
				case "reserved":
					return t(
						"thatNameIsReservedByFlowscript",
						"That name is reserved by FlowScript",
					);
				case "duplicate":
					return t(
						"aModuleWithThatNameAlreadyExistsHere",
						"A module with that name already exists here",
					);
				default:
					return null;
			}
		},
		[t],
	);

	const validateNew = useCallback(
		(value: string) =>
			nameErrorText(
				validateModuleName(value, board.layers, activeModuleId, reservedRoots),
			),
		[activeModuleId, board.layers, nameErrorText, reservedRoots],
	);

	const validateRename = useCallback(
		(value: string) =>
			nameErrorText(
				validateModuleName(
					value,
					board.layers,
					board.layers?.[renamingId ?? ""]?.parent_id ?? null,
					reservedRoots,
					renamingId ?? undefined,
				),
			),
		[board.layers, nameErrorText, renamingId, reservedRoots],
	);

	const commitCreate = useCallback(
		async (name: string) => {
			setDrafting(false);
			const layer = await createModule(name, activeModuleId);
			setPendingSelect(layer.id);
		},
		[activeModuleId, createModule],
	);

	const commitRename = useCallback(
		async (name: string) => {
			const layer = renamingId ? board.layers?.[renamingId] : undefined;
			setRenamingId(null);
			if (layer) await renameModule(layer, name);
		},
		[board.layers, renameModule, renamingId],
	);

	const removeModule = useCallback(
		async (moduleId: string, preserveNodes: boolean) => {
			const layer = board.layers?.[moduleId];
			if (!layer) return;
			await deleteModule(layer, preserveNodes);
		},
		[board.layers, deleteModule],
	);

	// A module being renamed keeps its tab even if it was closed meanwhile, so the inline
	// editor it is hosting does not vanish mid-edit.
	const renamingLabel = useMemo(
		() => (renamingId ? (board.layers?.[renamingId]?.name ?? "") : ""),
		[board.layers, renamingId],
	);
	const renamingHasTab = tabs.some(
		(tab) => tab.doc.kind === "board" && tab.doc.fileId === renamingId,
	);

	const pinnedTabs = tabs.filter((tab) => tab.pinned);
	const looseTabs = tabs.filter((tab) => !tab.pinned);

	const renderTab = (tab: IEditorTab) =>
		tab.doc.kind === "board" && renamingId === tab.doc.fileId ? (
			<ModuleNameField
				key={tab.key}
				initialValue={renamingLabel}
				submitLabel={t("rename", "Rename")}
				validate={validateRename}
				onSubmit={(name) => void commitRename(name)}
				onCancel={() => setRenamingId(null)}
			/>
		) : (
			<EditorTab
				key={tab.key}
				tab={tab}
				label={resolveLabel(tab)}
				active={activeKey === tab.key}
				closable={isTabClosable(tabs, tab.key)}
				readOnly={readOnly}
				actions={{
					select: () => onSelect(tab.key),
					close: () => onClose(tab.key),
					split: () => onSplit(tab.key),
					pin: (pinned) => onPin(tab.key, pinned),
					color: (color) => onColor(tab.key, color),
					rename: () =>
						setRenamingId(tab.doc.kind === "board" ? tab.doc.fileId : null),
					delete: () => {
						if (tab.doc.kind !== "board") return;
						void removeModule(tab.doc.fileId, true);
					},
					deleteWithContents: () => {
						if (tab.doc.kind !== "board") return;
						setPendingDelete({
							id: tab.doc.fileId,
							label: modulePathLabel(board.layers, tab.doc.fileId),
						});
					},
				}}
			/>
		);

	// Only the lanes scroll. Pinned tabs, the new-module control and the editor actions stay
	// put however many files are open.
	return (
		<>
			<nav
				aria-label={t("openEditors", "Open editors")}
				className="flex w-full shrink-0 items-center gap-1 border-b bg-muted/20 px-2"
			>
				{pinnedTabs.length > 0 && (
					<EditorTabLane
						activeKey={activeKey}
						itemsKey={pinnedTabs.map((tab) => tab.key).join("|")}
						className="max-w-[40%] shrink-0"
					>
						{pinnedTabs.map(renderTab)}
					</EditorTabLane>
				)}
				{pinnedTabs.length > 0 && looseTabs.length > 0 && (
					<span aria-hidden className="h-4 w-px shrink-0 bg-border" />
				)}
				<EditorTabLane
					activeKey={activeKey}
					itemsKey={looseTabs.map((tab) => tab.key).join("|")}
					className="min-w-0"
				>
					{looseTabs.map(renderTab)}
					{renamingId && !renamingHasTab && (
						<ModuleNameField
							initialValue={renamingLabel}
							submitLabel={t("rename", "Rename")}
							validate={validateRename}
							onSubmit={(name) => void commitRename(name)}
							onCancel={() => setRenamingId(null)}
						/>
					)}
				</EditorTabLane>
				{!readOnly &&
					(drafting ? (
						<ModuleNameField
							initialValue=""
							submitLabel={t("create", "Create")}
							validate={validateNew}
							onSubmit={(name) => void commitCreate(name)}
							onCancel={() => setDrafting(false)}
						/>
					) : (
						<Button
							size="icon"
							variant="ghost"
							className="size-6 shrink-0 text-muted-foreground"
							title={t("newModule", "New module")}
							aria-label={t("newModule", "New module")}
							onClick={() => setDrafting(true)}
						>
							<PlusIcon className="size-3.5" />
						</Button>
					))}
				{trailing && (
					<>
						<span className="flex-1" />
						{trailing}
					</>
				)}
			</nav>

			<AlertDialog
				open={pendingDelete !== null}
				onOpenChange={(open) => {
					if (!open) setPendingDelete(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t(
								"deleteModuleAndItsContents",
								"Delete module and its contents?",
							)}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"everythingInsideNameIsDeletedTooThisCannotBeUndone",
								"Everything inside {{name}} is deleted too. This cannot be undone.",
								{ name: pendingDelete?.label ?? "" },
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
						<AlertDialogAction
							className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
							onClick={() => {
								const target = pendingDelete;
								setPendingDelete(null);
								if (target) void removeModule(target.id, false);
							}}
						>
							{t("deleteWithContents", "Delete with contents")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}

/** `payments.flow`, or `payments.flow › processOrder` when the tab is parked in a layer. */
export function boardTabLabel(
	board: IBoard | undefined,
	fileId: string,
	layerPath: string | undefined,
): string {
	const file =
		fileId === MAIN_FILE_ID
			? MAIN_FILE_LABEL
			: modulePathLabel(board?.layers, fileId);
	const layerId = layerPath?.split("/").filter(Boolean).pop();
	if (!layerId || layerId === fileId) return file;
	const name = board?.layers?.[layerId]?.name;
	return name ? `${file} › ${name}` : file;
}
