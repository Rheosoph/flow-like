"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangleIcon,
	CheckCircle2Icon,
	ChevronDownIcon,
	ChevronUpIcon,
	Loader2Icon,
} from "lucide-react";
import { useMemo } from "react";
import type { BoardCommand } from "../../../lib/schema/flow/copilot";
import type { IFlowScriptSourceSpan } from "../../../state/backend-state/board-state";
import { FlowScriptDiagnosticList } from "./flowscript-diagnostic-list";
import {
	type FlowScriptDiagnosticEntry,
	flowScriptDiagnosticEntries,
} from "./flowscript-diagnostics";

/**
 * The Rust `BoardCommand` enum has variants the hand-mirrored TS union lags
 * behind on (e.g. `RemoveLayer`), so the preview reads commands structurally
 * and treats unknown command types as generic updates instead of breaking.
 */
interface AnyBoardCommand {
	command_type: string;
	summary?: string;
	node_id?: string;
	variable_id?: string;
	layer_id?: string;
	comment_id?: string;
	ids?: string[];
	target_layer?: string | null;
	name?: string;
	friendly_name?: string;
	node_type?: string;
}

export interface ApplyPreviewCounts {
	addedNodes: number;
	removedNodes: number;
	wires: number;
	removedWires: number;
	variables: number;
	layers: number;
	comments: number;
	updates: number;
	total: number;
}

export function summarizeBoardCommands(
	commands: readonly BoardCommand[],
): ApplyPreviewCounts {
	const counts: ApplyPreviewCounts = {
		addedNodes: 0,
		removedNodes: 0,
		wires: 0,
		removedWires: 0,
		variables: 0,
		layers: 0,
		comments: 0,
		updates: 0,
		total: commands.length,
	};
	for (const command of commands as readonly AnyBoardCommand[]) {
		switch (command.command_type) {
			case "AddNode":
			case "AddPlaceholder":
				counts.addedNodes++;
				break;
			case "RemoveNode":
				counts.removedNodes++;
				break;
			case "ConnectPins":
				counts.wires++;
				break;
			case "DisconnectPins":
				counts.removedWires++;
				break;
			case "CreateVariable":
			case "UpdateVariable":
			case "RemoveVariable":
			case "DeleteVariable":
				counts.variables++;
				break;
			case "CreateLayer":
			case "RemoveLayer":
			case "UpdateLayerCache":
			case "AddNodesToLayer":
			case "RemoveNodesFromLayer":
				counts.layers++;
				break;
			case "CreateComment":
			case "UpdateComment":
			case "RemoveComment":
			case "DeleteComment":
				counts.comments++;
				break;
			default:
				counts.updates++;
				break;
		}
	}
	return counts;
}

export interface DestructiveCommandSummary {
	kind: "node" | "variable" | "layer" | "comment";
	label: string;
}

const DESTRUCTIVE_KINDS: Record<string, DestructiveCommandSummary["kind"]> = {
	RemoveNode: "node",
	RemoveVariable: "variable",
	DeleteVariable: "variable",
	RemoveLayer: "layer",
	RemoveComment: "comment",
	DeleteComment: "comment",
};

/** The nodes, variables, layers, and comments a scoped/full apply would delete. */
export function destructiveCommandSummaries(
	commands: readonly BoardCommand[],
): DestructiveCommandSummary[] {
	const summaries: DestructiveCommandSummary[] = [];
	for (const command of commands as readonly AnyBoardCommand[]) {
		const kind = DESTRUCTIVE_KINDS[command.command_type];
		if (!kind) continue;
		const label =
			command.summary ??
			command.node_id ??
			command.variable_id ??
			command.layer_id ??
			command.comment_id ??
			command.command_type;
		summaries.push({ kind, label });
	}
	return summaries;
}

export interface ApplyDeletionItem extends DestructiveCommandSummary {
	key: string;
	id?: string;
}

export interface ApplyMoveItem {
	key: string;
	id: string;
	label: string;
	/** Destination layer ID; null or undefined means the board root. */
	targetLayer?: string | null;
}

export interface ApplyImpact {
	deletions: ApplyDeletionItem[];
	moves: ApplyMoveItem[];
}

/** Count affected items once and retain every item for confirmation review. */
export function summarizeApplyImpact(
	commands: readonly BoardCommand[],
): ApplyImpact {
	const deletions = new Map<string, ApplyDeletionItem>();
	const moves = new Map<string, ApplyMoveItem>();
	const deletedIds = new Set<string>();
	for (const [index, command] of (
		commands as readonly AnyBoardCommand[]
	).entries()) {
		const kind = DESTRUCTIVE_KINDS[command.command_type];
		if (kind) {
			const id =
				kind === "node"
					? command.node_id
					: kind === "variable"
						? command.variable_id
						: kind === "layer"
							? command.layer_id
							: command.comment_id;
			const key = id ? `${kind}:${id}` : `${kind}:command:${index}`;
			deletions.set(key, {
				key,
				kind,
				id,
				label: command.summary ?? id ?? command.command_type,
			});
			if (id) deletedIds.add(id);
		} else if (command.command_type === "MoveToLayer") {
			for (const id of command.ids ?? []) {
				moves.set(id, {
					key: `move:${id}`,
					id,
					label: command.summary ?? id,
					targetLayer: command.target_layer,
				});
			}
		}
	}
	return {
		deletions: [...deletions.values()],
		moves: [...moves.values()].filter((item) => !deletedIds.has(item.id)),
	};
}

export interface BoardCommandGroupItem {
	/** Stable within the group even when two commands share a label. */
	key: string;
	label: string;
	/** Board entity the command targets, when derivable (remote-touched marking). */
	entityId?: string;
}

export interface BoardCommandGroup {
	type: string;
	items: BoardCommandGroupItem[];
}

/** Commands grouped by type, in first-seen order, labeled by their compiler summary. */
export function groupBoardCommands(
	commands: readonly BoardCommand[],
): BoardCommandGroup[] {
	const groups = new Map<string, BoardCommandGroupItem[]>();
	const labelCounts = new Map<string, number>();
	for (const command of commands as readonly AnyBoardCommand[]) {
		const label =
			command.summary ??
			command.friendly_name ??
			command.name ??
			command.node_type ??
			command.node_id ??
			command.variable_id ??
			command.layer_id ??
			command.comment_id ??
			command.command_type;
		const labelKey = `${command.command_type}\u0000${label}`;
		const occurrence = (labelCounts.get(labelKey) ?? 0) + 1;
		labelCounts.set(labelKey, occurrence);
		const entityId =
			command.node_id ?? command.variable_id ?? command.layer_id ?? undefined;
		const item: BoardCommandGroupItem = {
			key: `${label}#${occurrence}`,
			label,
			...(entityId ? { entityId } : {}),
		};
		const bucket = groups.get(command.command_type);
		if (bucket) bucket.push(item);
		else groups.set(command.command_type, [item]);
	}
	return [...groups.entries()].map(([type, items]) => ({ type, items }));
}

export interface FlowScriptApplyPreviewState {
	/** Undefined until a reconcile check for the current text finished. */
	counts?: ApplyPreviewCounts;
	errorCount: number;
	warningCount?: number;
	destructiveCount: number;
	checking: boolean;
	/**
	 * Locally edited statements that ALSO changed remotely since the local
	 * baseline — applying overwrites the remote version (last-writer-wins).
	 */
	remoteOverlapCount?: number;
}

export function FlowScriptApplyPreviewChip({
	state,
	expanded,
	onToggle,
}: Readonly<{
	state: FlowScriptApplyPreviewState;
	expanded: boolean;
	onToggle: () => void;
}>) {
	const { t } = useTranslation("flow");
	const { counts, errorCount, destructiveCount, checking } = state;
	const remoteOverlapCount = state.remoteOverlapCount ?? 0;
	const warningCount = state.warningCount ?? 0;

	const segments = useMemo(() => {
		if (!counts) return [];
		const parts: string[] = [];
		if (counts.addedNodes > 0)
			parts.push(
				t("flowscriptPreviewAddedNodes", {
					defaultValue_one: "+{{count}} node",
					defaultValue_other: "+{{count}} nodes",
					count: counts.addedNodes,
				}),
			);
		if (counts.removedNodes > 0)
			parts.push(
				t("flowscriptPreviewRemovedNodes", {
					defaultValue_one: "−{{count}} node",
					defaultValue_other: "−{{count}} nodes",
					count: counts.removedNodes,
				}),
			);
		if (counts.wires > 0)
			parts.push(
				t("flowscriptPreviewWires", {
					defaultValue_one: "{{count}} wire",
					defaultValue_other: "{{count}} wires",
					count: counts.wires,
				}),
			);
		if (counts.removedWires > 0)
			parts.push(
				t("flowscriptPreviewRemovedWires", {
					defaultValue_one: "−{{count}} wire",
					defaultValue_other: "−{{count}} wires",
					count: counts.removedWires,
				}),
			);
		if (counts.variables > 0)
			parts.push(
				t("flowscriptPreviewVariables", {
					defaultValue_one: "{{count}} var",
					defaultValue_other: "{{count}} vars",
					count: counts.variables,
				}),
			);
		if (counts.layers > 0)
			parts.push(
				t("flowscriptPreviewLayers", {
					defaultValue_one: "{{count}} layer",
					defaultValue_other: "{{count}} layers",
					count: counts.layers,
				}),
			);
		if (counts.comments > 0)
			parts.push(
				t("flowscriptPreviewComments", {
					defaultValue_one: "{{count}} comment",
					defaultValue_other: "{{count}} comments",
					count: counts.comments,
				}),
			);
		if (counts.updates > 0)
			parts.push(
				t("flowscriptPreviewUpdates", {
					defaultValue_one: "{{count}} update",
					defaultValue_other: "{{count}} updates",
					count: counts.updates,
				}),
			);
		return parts;
	}, [counts, t]);

	if (!counts && !checking && errorCount === 0 && warningCount === 0)
		return null;

	const hasErrors = errorCount > 0;
	const noChanges =
		!hasErrors && warningCount === 0 && (counts?.total ?? 0) === 0;
	const warningSegment =
		warningCount > 0
			? t("countWarnings", {
					defaultValue_one: "{{count}} warning",
					defaultValue_other: "{{count}} warnings",
					count: warningCount,
				})
			: undefined;
	const remoteOverlapSegment =
		remoteOverlapCount > 0
			? `⚠ ${t("flowscriptPreviewRemoteOverlap", {
					defaultValue_one: "{{count}} remote",
					defaultValue_other: "{{count}} remote",
					count: remoteOverlapCount,
				})}`
			: undefined;

	return (
		<button
			type="button"
			onClick={onToggle}
			className={`flex h-6 max-w-full items-center gap-1 truncate rounded-full border px-2 text-[10px] font-medium transition-colors ${
				hasErrors
					? "border-destructive/40 text-destructive"
					: destructiveCount > 0 || remoteOverlapCount > 0 || warningCount > 0
						? "border-yellow-500/50 text-yellow-600 dark:text-yellow-400"
						: "border-border text-muted-foreground hover:text-foreground"
			}`}
			title={
				remoteOverlapSegment
					? t(
							"flowscriptPreviewRemoteOverlapTitle",
							"Statements you edited also changed remotely — applying overwrites the remote version",
						)
					: t("flowscriptPreviewToggle", "Show pending FlowScript changes")
			}
		>
			{checking ? (
				<Loader2Icon className="h-3 w-3 shrink-0 animate-spin" />
			) : hasErrors || warningCount > 0 ? (
				<AlertTriangleIcon className="h-3 w-3 shrink-0" />
			) : (
				<CheckCircle2Icon className="h-3 w-3 shrink-0" />
			)}
			<span className="truncate">
				{hasErrors
					? t("flowscriptPreviewErrors", {
							defaultValue_one: "{{count}} error",
							defaultValue_other: "{{count}} errors",
							count: errorCount,
						})
					: noChanges
						? t("flowscriptPreviewNoChanges", "No changes")
						: [
								...segments,
								...(warningSegment ? [warningSegment] : []),
								...(remoteOverlapSegment ? [remoteOverlapSegment] : []),
							].join(" · ")}
			</span>
			{expanded ? (
				<ChevronUpIcon className="h-3 w-3 shrink-0" />
			) : (
				<ChevronDownIcon className="h-3 w-3 shrink-0" />
			)}
		</button>
	);
}

export function FlowScriptApplyPreviewList({
	commands,
	diagnostics,
	warnings = [],
	diagnosticEntries,
	source,
	onRevealDiagnostic,
	remoteTouchedIds,
}: Readonly<{
	commands: readonly BoardCommand[];
	diagnostics: readonly string[];
	warnings?: readonly string[];
	diagnosticEntries?: readonly FlowScriptDiagnosticEntry[];
	source?: string;
	onRevealDiagnostic?: (span: IFlowScriptSourceSpan) => void;
	/** Entity ids changed remotely since the local baseline (marked per item). */
	remoteTouchedIds?: ReadonlySet<string>;
}>) {
	const { t } = useTranslation("flow");
	const groups = useMemo(() => groupBoardCommands(commands), [commands]);

	return (
		<div className="max-h-40 shrink-0 overflow-y-auto border-b px-3 py-2">
			{diagnostics.length > 0 && (
				<div className="mb-2 text-destructive">
					<FlowScriptDiagnosticList
						entries={
							diagnosticEntries ?? flowScriptDiagnosticEntries(diagnostics)
						}
						source={source}
						onReveal={onRevealDiagnostic}
					/>
				</div>
			)}
			{warnings.length > 0 && (
				<div className="mb-2 text-yellow-600 dark:text-yellow-400">
					<FlowScriptDiagnosticList
						entries={flowScriptDiagnosticEntries(warnings)}
					/>
				</div>
			)}
			{groups.length === 0 && diagnostics.length === 0 && (
				<p className="text-xs text-muted-foreground">
					{t(
						"flowscriptPreviewInSync",
						"Applying this text would not change the board.",
					)}
				</p>
			)}
			{groups.map((group) => (
				<div key={group.type} className="mb-2 last:mb-0">
					<span className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
						{group.type} · {group.items.length}
					</span>
					<ul className="mt-0.5 space-y-0.5">
						{group.items.map((item) => {
							const remoteTouched = Boolean(
								item.entityId && remoteTouchedIds?.has(item.entityId),
							);
							return (
								<li
									key={item.key}
									className="flex items-center gap-1.5 text-xs text-foreground/80"
									title={item.label}
								>
									<span className="truncate">{item.label}</span>
									{remoteTouched && (
										<span className="shrink-0 rounded-full border border-yellow-500/50 px-1.5 text-[9px] font-medium text-yellow-600 dark:text-yellow-400">
											{t("flowscriptPreviewRemoteTouched", "changed remotely")}
										</span>
									)}
								</li>
							);
						})}
					</ul>
				</div>
			))}
		</div>
	);
}
