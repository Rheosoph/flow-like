"use client";

import { useTranslation } from "@flow-like/locales";
import {
	FileCode2Icon,
	PencilLineIcon,
	PlusIcon,
	Trash2Icon,
	XIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { IGenericCommand } from "../../lib";
import {
	FLOWSCRIPT_KEYWORDS,
	type IBoardModule,
	type IModuleNameError,
	MAIN_FILE_LABEL,
	boardModules,
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
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "../ui/context-menu";
import { Input } from "../ui/input";
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

	return (
		<div className="relative flex shrink-0 items-center gap-1">
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
			{error && (
				<span className="absolute top-full left-0 z-50 mt-1 whitespace-nowrap rounded-md border border-destructive/40 bg-popover px-2 py-1 text-[10px] text-destructive shadow-md">
					{error}
				</span>
			)}
		</div>
	);
}

function ModuleTabButton({
	label,
	active,
	onSelect,
	onClose,
}: Readonly<{
	label: string;
	active: boolean;
	onSelect: () => void;
	/** Absent for `main.flow`, which is the board itself and always open. */
	onClose?: () => void;
}>) {
	const { t } = useTranslation("flow");
	return (
		<span
			className={cn(
				"group/tab flex shrink-0 items-center rounded-md border pr-1 transition-colors",
				active
					? "border-border bg-background text-foreground shadow-sm"
					: "border-transparent text-muted-foreground hover:bg-muted/60 hover:text-foreground",
			)}
		>
			<button
				type="button"
				onClick={onSelect}
				title={label}
				aria-current={active ? "page" : undefined}
				className="flex items-center gap-1.5 py-1 pl-2.5 pr-1 font-mono text-xs"
			>
				<FileCode2Icon className="size-3 shrink-0" />
				<span className="max-w-48 truncate">{label}</span>
			</button>
			{onClose && (
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
			)}
		</span>
	);
}

function ModuleTab({
	module,
	active,
	readOnly,
	onSelect,
	onClose,
	onRename,
	onDelete,
	onDeleteWithContents,
}: Readonly<{
	module: IBoardModule;
	active: boolean;
	readOnly: boolean;
	onSelect: () => void;
	onClose: () => void;
	onRename: () => void;
	onDelete: () => void;
	onDeleteWithContents: () => void;
}>) {
	const { t } = useTranslation("flow");
	const tab = (
		<ModuleTabButton
			label={module.pathLabel}
			active={active}
			onSelect={onSelect}
			onClose={onClose}
		/>
	);

	if (readOnly) return tab;

	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>{tab}</ContextMenuTrigger>
			<ContextMenuContent className="w-56">
				<ContextMenuItem onSelect={onClose}>
					<XIcon className="size-3.5" />
					{t("closeFile", "Close file")}
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem onSelect={onRename}>
					<PencilLineIcon className="size-3.5" />
					{t("rename", "Rename")}
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem onSelect={onDelete}>
					<Trash2Icon className="size-3.5" />
					{t("deleteModule", "Delete module")}
				</ContextMenuItem>
				<ContextMenuItem variant="destructive" onSelect={onDeleteWithContents}>
					<Trash2Icon className="size-3.5" />
					{t("deleteWithContents", "Delete with contents")}
				</ContextMenuItem>
			</ContextMenuContent>
		</ContextMenu>
	);
}

/**
 * The board's files: `main.flow` plus one tab per module layer. Selecting a tab opens that
 * module on the canvas, which is nothing more than making it the current layer — modules are
 * organizational, so their nodes render exactly as the root's do.
 */
export function FlowModuleTabs({
	board,
	activeModuleId,
	openFileIds,
	onSelect,
	onCloseFile,
	executeCommand,
	readOnly,
	reservedRoots = FLOWSCRIPT_KEYWORDS,
	trailing,
}: Readonly<{
	board: IBoard;
	activeModuleId: string | null;
	/**
	 * Module ids with a tab. The strip shows what is *open*, the explorer shows
	 * what *exists* — so a tab can be closed and reopened by clicking the file.
	 */
	openFileIds: readonly string[];
	onSelect: (moduleId: string | null) => void;
	onCloseFile: (moduleId: string) => void;
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

	const allModules = useMemo(() => boardModules(board.layers), [board.layers]);
	const [drafting, setDrafting] = useState(false);
	const [renamingId, setRenamingId] = useState<string | null>(null);
	const [pendingDelete, setPendingDelete] = useState<IBoardModule | null>(null);
	const [pendingSelect, setPendingSelect] = useState<string | null>(null);

	const open = useMemo(() => new Set(openFileIds), [openFileIds]);
	// A module being renamed keeps its tab even if it was never opened, so the
	// inline editor it is hosting does not vanish mid-edit.
	const modules = useMemo(
		() =>
			allModules.filter(
				(module) => open.has(module.id) || renamingId === module.id,
			),
		[allModules, open, renamingId],
	);

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
			// The tab is about to disappear — leave the user on a file that still exists.
			if (activeModuleId === moduleId) onSelect(layer.parent_id ?? null);
			await deleteModule(layer, preserveNodes);
		},
		[activeModuleId, board.layers, deleteModule, onSelect],
	);

	return (
		<>
			<nav
				aria-label={t("modules", "Modules")}
				className="no-scrollbar flex w-full shrink-0 items-center gap-1 overflow-x-auto border-b bg-muted/20 px-2 py-1"
			>
				<ModuleTabButton
					label={MAIN_FILE_LABEL}
					active={activeModuleId === null}
					onSelect={() => onSelect(null)}
				/>
				{modules.map((module) =>
					renamingId === module.id ? (
						<ModuleNameField
							key={module.id}
							initialValue={module.name}
							submitLabel={t("rename", "Rename")}
							validate={validateRename}
							onSubmit={(name) => void commitRename(name)}
							onCancel={() => setRenamingId(null)}
						/>
					) : (
						<ModuleTab
							key={module.id}
							module={module}
							active={activeModuleId === module.id}
							readOnly={readOnly}
							onSelect={() => onSelect(module.id)}
							onClose={() => onCloseFile(module.id)}
							onRename={() => setRenamingId(module.id)}
							onDelete={() => void removeModule(module.id, true)}
							onDeleteWithContents={() => setPendingDelete(module)}
						/>
					),
				)}
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
								{ name: pendingDelete?.pathLabel ?? "" },
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
