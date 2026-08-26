"use client";

import { useTranslation } from "@flow-like/locales";
import { FileCode2Icon } from "lucide-react";
import {
	type IBoardModule,
	MAIN_FILE_ID,
	MAIN_FILE_LABEL,
} from "../../../lib/flow-modules";
import { cn } from "../../../lib/utils";

/** One document tab: a virtual FlowScript file of the board. */
export interface FlowScriptFileTab {
	/** `"main"` or the module layer id. */
	id: string;
	/** `main.flow`, `checkout/payments.flow`. */
	label: string;
	/** The file holds edits that were not applied to the board yet. */
	dirty: boolean;
}

/**
 * The board's files as tabs, in the order the user reads them: `main.flow` first, then every
 * module by path. Dirty state comes from the panel for the open file and from the stash for the
 * rest, so a tab the user left mid-edit keeps its dot.
 */
export function flowScriptFileTabs(
	modules: readonly IBoardModule[],
	activeFileId: string,
	activeDirty: boolean,
	stashedDirtyFileIds: ReadonlySet<string>,
): FlowScriptFileTab[] {
	const dirtyOf = (fileId: string) =>
		fileId === activeFileId ? activeDirty : stashedDirtyFileIds.has(fileId);
	return [
		{
			id: MAIN_FILE_ID,
			label: MAIN_FILE_LABEL,
			dirty: dirtyOf(MAIN_FILE_ID),
		},
		...modules.map((module) => ({
			id: module.id,
			label: module.pathLabel,
			dirty: dirtyOf(module.id),
		})),
	];
}

function FlowScriptFileTabButton({
	tab,
	active,
	disabled,
	onSelect,
}: Readonly<{
	tab: FlowScriptFileTab;
	active: boolean;
	disabled: boolean;
	onSelect: () => void;
}>) {
	const { t } = useTranslation("flow");
	const dirtyLabel = t("flowscriptFileUnapplied", {
		defaultValue: "{{file}} has unapplied changes",
		file: tab.label,
	});
	return (
		<button
			type="button"
			disabled={disabled}
			onClick={onSelect}
			title={tab.dirty ? dirtyLabel : tab.label}
			aria-current={active ? "page" : undefined}
			aria-label={t("flowscriptOpenFile", {
				defaultValue: "Open {{file}}",
				file: tab.label,
			})}
			className={cn(
				"flex h-6 shrink-0 items-center gap-1.5 rounded-md border px-2 font-mono text-[11px] transition-colors disabled:pointer-events-none disabled:opacity-60",
				active
					? "border-border bg-background text-foreground shadow-sm"
					: "border-transparent text-muted-foreground hover:bg-muted/60 hover:text-foreground",
			)}
		>
			<FileCode2Icon className="size-3 shrink-0" />
			<span className="max-w-40 truncate">{tab.label}</span>
			{tab.dirty && (
				<span
					aria-label={dirtyLabel}
					className="size-1.5 shrink-0 rounded-full bg-primary"
				/>
			)}
		</button>
	);
}

/**
 * The FlowScript panel's file strip. It owns no selection of its own: picking a tab opens that
 * module on the canvas through the board's module-select path, and the canvas decides which file
 * is current. Creating, renaming and deleting modules stays on the canvas strip.
 */
export function FlowScriptFileTabs({
	tabs,
	activeFileId,
	disabled,
	onSelect,
}: Readonly<{
	tabs: readonly FlowScriptFileTab[];
	activeFileId: string;
	disabled?: boolean;
	/** `null` selects `main`; anything else is a module layer id. */
	onSelect: (moduleId: string | null) => void;
}>) {
	const { t } = useTranslation("flow");

	if (tabs.length < 2) return null;

	return (
		<nav
			aria-label={t("flowscriptFiles", "FlowScript files")}
			className="no-scrollbar flex w-full shrink-0 items-center gap-1 overflow-x-auto border-b bg-muted/20 px-2 py-1"
		>
			{tabs.map((tab) => (
				<FlowScriptFileTabButton
					key={tab.id}
					tab={tab}
					active={tab.id === activeFileId}
					disabled={disabled ?? false}
					onSelect={() => onSelect(tab.id === MAIN_FILE_ID ? null : tab.id)}
				/>
			))}
		</nav>
	);
}
