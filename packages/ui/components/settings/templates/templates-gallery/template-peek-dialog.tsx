"use client";

import { useTranslation } from "@flow-like/locales";
import {
	LoaderCircleIcon,
	PencilIcon,
	PlusIcon,
	Trash2Icon,
	WorkflowIcon,
} from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import { formatAbsoluteDateTime } from "../../../../lib/date";
import { cn } from "../../../../lib/utils";
import { FlowPreview } from "../../../flow/flow-preview";
import { Button } from "../../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogTitle,
} from "../../../ui/dialog";
import { Input } from "../../../ui/input";
import { Label } from "../../../ui/label";
import { Skeleton } from "../../../ui/skeleton";
import { PREVIEW_SURFACE } from "./template-card";
import {
	type ITemplateItem,
	defaultFlowName,
	versionLabel,
} from "./templates-gallery-model";

const MAX_NODE_TYPES = 12;

export interface ITemplatePeek {
	readonly item: ITemplateItem;
	/** Opened from a card's "New flow" button: show the name field right away. */
	readonly startWithFlowForm: boolean;
}

interface TemplatePeekDialogProps {
	readonly peek: ITemplatePeek | null;
	readonly onClose: () => void;
	readonly canCreateFlow: boolean;
	readonly canDelete: boolean;
	/** Resolves false when the caller already reported a failure. */
	readonly onCreateFlow: (
		item: ITemplateItem,
		name: string,
	) => Promise<boolean>;
	readonly onEdit: (item: ITemplateItem) => void;
	/** Resolves false when the caller already reported a failure. */
	readonly onDelete: (item: ITemplateItem) => Promise<boolean>;
}

export function TemplatePeekDialog({
	peek,
	onClose,
	...actions
}: TemplatePeekDialogProps) {
	return (
		<Dialog open={peek !== null} onOpenChange={(open) => !open && onClose()}>
			<DialogContent className="max-h-[calc(100dvh-3rem)] gap-0 overflow-y-auto p-0 sm:max-w-2xl">
				{peek ? (
					<PeekBody
						key={`${peek.item.key}:${peek.startWithFlowForm}`}
						peek={peek}
						onClose={onClose}
						{...actions}
					/>
				) : null}
			</DialogContent>
		</Dialog>
	);
}

function PeekBody({
	peek,
	onClose,
	canCreateFlow,
	canDelete,
	onCreateFlow,
	onEdit,
	onDelete,
}: Readonly<{ peek: ITemplatePeek } & Omit<TemplatePeekDialogProps, "peek">>) {
	const { t } = useTranslation("common");
	const { item } = peek;
	const digest = item.digest;
	const [flowFormOpen, setFlowFormOpen] = useState(
		peek.startWithFlowForm && canCreateFlow,
	);
	const [flowName, setFlowName] = useState(() => defaultFlowName(item));
	const [creating, setCreating] = useState(false);
	const [confirmDelete, setConfirmDelete] = useState(false);
	const [deleting, setDeleting] = useState(false);
	const nameId = useId();
	const version = versionLabel(digest?.version);
	const extraTypes = Math.max(
		0,
		(digest?.nodeTypes.length ?? 0) - MAX_NODE_TYPES,
	);

	const submitFlow = async (event: FormEvent) => {
		event.preventDefault();
		if (creating) return;
		setCreating(true);
		const created = await onCreateFlow(
			item,
			flowName.trim() || defaultFlowName(item),
		);
		setCreating(false);
		if (created) onClose();
	};

	const remove = async () => {
		if (deleting) return;
		setDeleting(true);
		const deleted = await onDelete(item);
		setDeleting(false);
		if (deleted) onClose();
	};

	return (
		<>
			<div
				className={cn(
					"relative h-64 shrink-0 overflow-hidden border-b border-border/60",
					PREVIEW_SURFACE,
				)}
			>
				{item.board ? (
					<FlowPreview
						nodes={Object.values(item.board.nodes ?? {})}
						comments={item.board.comments}
						layers={item.board.layers}
						variables={item.board.variables}
					/>
				) : item.boardState === "loading" ? (
					<Skeleton aria-hidden className="m-4 h-[calc(100%-2rem)]" />
				) : (
					<div className="flex size-full items-center justify-center text-sm text-muted-foreground">
						{t("previewNotAvailable", "Preview not available")}
					</div>
				)}
			</div>

			<div className="flex flex-col gap-5 p-6">
				<div className="flex flex-col gap-1.5 pr-6">
					<p className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground">
						<WorkflowIcon aria-hidden className="size-3.5" />
						{item.sourceName ? (
							<span className="font-medium text-foreground">
								{item.sourceName}
							</span>
						) : null}
						{version ? (
							<span className="font-mono text-[11px]">{version}</span>
						) : null}
						{item.updatedAtMs ? (
							<>
								<span aria-hidden>·</span>
								<span>{formatAbsoluteDateTime(item.updatedAtMs)}</span>
							</>
						) : null}
					</p>
					<DialogTitle className="text-xl font-semibold leading-snug wrap-break-word">
						{item.name}
					</DialogTitle>
					{item.description ? (
						<DialogDescription className="text-sm text-muted-foreground">
							{item.description}
						</DialogDescription>
					) : (
						<DialogDescription className="sr-only">
							{t(
								"templatePeekDescription",
								"Preview of the template and what it contains.",
							)}
						</DialogDescription>
					)}
				</div>

				<dl className="grid grid-cols-2 gap-2 sm:grid-cols-4">
					<StatTile
						label={t("nodes", "Nodes")}
						value={digest?.nodeCount}
						loading={item.boardState === "loading"}
					/>
					<StatTile
						label={t("layers", "Layers")}
						value={digest?.layerCount}
						loading={item.boardState === "loading"}
					/>
					<StatTile
						label={t("variables", "Variables")}
						value={digest?.variableCount}
						loading={item.boardState === "loading"}
					/>
					<StatTile
						label={t("templateStartsOn", "Starts on")}
						value={
							digest
								? digest.entryNames.join(", ") ||
									t("templateNoEntryEvent", "No entry event")
								: undefined
						}
						loading={item.boardState === "loading"}
						small
					/>
				</dl>

				{digest && digest.nodeTypes.length > 0 ? (
					<section className="flex flex-col gap-2">
						<h3 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
							{t("templateNodeTypes", "Node types")}
						</h3>
						<ul className="flex flex-wrap gap-1.5">
							{digest.nodeTypes.slice(0, MAX_NODE_TYPES).map((type) => (
								<li
									key={type}
									className="rounded-full border border-border/70 px-2.5 py-0.5 text-xs text-foreground/80"
								>
									{type}
								</li>
							))}
							{extraTypes > 0 ? (
								<li className="rounded-full px-2 py-0.5 text-xs text-muted-foreground">
									{t("templateMoreNodeTypes", "+{{count}} more", {
										count: extraTypes,
										defaultValue_one: "+{{count}} more",
									})}
								</li>
							) : null}
						</ul>
					</section>
				) : null}

				{flowFormOpen ? (
					<form
						onSubmit={submitFlow}
						className="flex flex-col gap-2 rounded-lg border border-border/60 bg-muted/30 p-3.5"
					>
						<Label htmlFor={nameId} className="text-sm font-medium">
							{t("templateNewFlowName", "Name of the new flow")}
						</Label>
						<div className="flex flex-wrap gap-2">
							<Input
								id={nameId}
								autoFocus
								value={flowName}
								onChange={(event) => setFlowName(event.target.value)}
								className="h-9 min-w-0 flex-1 basis-60 bg-background"
							/>
							<Button type="submit" disabled={creating} className="h-9">
								{creating ? (
									<LoaderCircleIcon
										aria-hidden
										className="size-4 animate-spin"
									/>
								) : null}
								{t("createFlow", "Create Flow")}
							</Button>
						</div>
						<p className="text-xs text-muted-foreground">
							{t(
								"templateNewFlowHint",
								"The new flow gets its own copy of this template. The template stays unchanged.",
							)}
						</p>
					</form>
				) : null}

				{confirmDelete ? (
					<div
						role="alert"
						className="flex flex-col gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-3.5"
					>
						<p className="text-sm wrap-break-word">
							{t(
								"templateDeleteConfirm",
								"Delete “{{name}}”? Flows already created from it stay as they are.",
								{ name: item.name },
							)}
						</p>
						<div className="flex justify-end gap-2">
							<Button
								variant="outline"
								size="sm"
								onClick={() => setConfirmDelete(false)}
								disabled={deleting}
							>
								{t("cancel", "Cancel")}
							</Button>
							<Button
								variant="destructive"
								size="sm"
								onClick={remove}
								disabled={deleting}
							>
								{deleting ? (
									<LoaderCircleIcon
										aria-hidden
										className="size-4 animate-spin"
									/>
								) : null}
								{t("delete", "Delete")}
							</Button>
						</div>
					</div>
				) : null}

				<div className="flex flex-wrap items-center justify-between gap-2">
					<div className="flex flex-wrap gap-2">
						{canCreateFlow && !flowFormOpen && item.board ? (
							<Button onClick={() => setFlowFormOpen(true)}>
								<PlusIcon aria-hidden className="size-4" />
								{t("newFlowFromTemplate", "New flow from template")}
							</Button>
						) : null}
						<Button variant="outline" onClick={() => onEdit(item)}>
							<PencilIcon aria-hidden className="size-4" />
							{t("editDetails", "Edit details")}
						</Button>
					</div>
					{canDelete && !confirmDelete ? (
						<Button
							variant="ghost"
							className="text-destructive hover:bg-destructive/10 hover:text-destructive"
							onClick={() => setConfirmDelete(true)}
						>
							<Trash2Icon aria-hidden className="size-4" />
							{t("delete", "Delete")}
						</Button>
					) : null}
				</div>
			</div>
		</>
	);
}

function StatTile({
	label,
	value,
	loading,
	small,
}: Readonly<{
	label: string;
	value: number | string | undefined;
	loading: boolean;
	small?: boolean;
}>) {
	return (
		<div className="flex min-w-0 flex-col gap-0.5 rounded-lg border border-border/60 bg-muted/30 px-3 py-2.5">
			<dt className="text-xs text-muted-foreground">{label}</dt>
			<dd
				className={cn(
					"font-semibold",
					small
						? "line-clamp-2 text-sm leading-5 wrap-break-word"
						: "truncate text-lg leading-6 tabular-nums",
				)}
				title={typeof value === "string" ? value : undefined}
			>
				{value ??
					(loading ? <Skeleton aria-hidden className="mt-1 h-4 w-10" /> : "—")}
			</dd>
		</div>
	);
}
