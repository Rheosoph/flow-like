"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation } from "@tanstack/react-query";
import {
	ArrowLeftIcon,
	CheckIcon,
	GitMergeIcon,
	PencilIcon,
	Trash2Icon,
	UsersIcon,
	XIcon,
} from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { getErrorMessage } from "../../lib/error-message";
import { cn } from "../../lib/utils";
import {
	type BotEntity,
	type BotFact,
	type BotFactsOwner,
	forgetBotEntity,
	mergeBotEntity,
	updateBotEntity,
} from "../../state/global-chat/bot-memory-facts";
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
	Command,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
} from "../ui/command";
import { Input } from "../ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { BotEntityGraph } from "./bot-entity-graph";
import { BotMemoryCard } from "./bot-memory-card";
import {
	ENTITY_KINDS,
	egoGraph,
	entityAliases,
	entityFacts,
	entityKey,
	entityLabel,
	mergeTargets,
} from "./bot-memory-entities-model";
import { entityStyle, useEntityKindName } from "./bot-memory-entity-kinds";
import type { MemoryFormat } from "./bot-memory-model";

export interface EntityFactActions {
	now: number;
	format: MemoryFormat;
	pendingId: string | null;
	disabled: boolean;
	onForget: (id: string) => void;
	onJump: (seq: number) => void;
}

function RenameField({
	entity,
	label,
	busy,
	onSave,
}: Readonly<{
	entity: BotEntity;
	label: string;
	busy: boolean;
	onSave: (name: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const [draft, setDraft] = useState<string | null>(null);
	const rename = t("memory.entities.rename", "Rename");
	if (entity.owner) {
		return (
			<h4 className="min-w-0 truncate text-base font-semibold tracking-tight">
				{label}
			</h4>
		);
	}
	const commit = () => {
		const name = draft?.trim();
		setDraft(null);
		if (name && name !== entity.name) onSave(name);
	};
	if (draft === null) {
		return (
			<div className="flex min-w-0 items-center gap-1">
				<h4 className="min-w-0 wrap-anywhere text-base font-semibold tracking-tight">
					{label}
				</h4>
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="size-7 shrink-0 rounded-lg text-muted-foreground"
					aria-label={rename}
					title={rename}
					disabled={busy}
					onClick={() => setDraft(entity.name)}
				>
					<PencilIcon className="size-3.5" aria-hidden />
				</Button>
			</div>
		);
	}
	return (
		<form
			className="flex min-w-0 items-center gap-1"
			onSubmit={(event) => {
				event.preventDefault();
				commit();
			}}
		>
			<Input
				autoFocus
				value={draft}
				onChange={(event) => setDraft(event.target.value)}
				onKeyDown={(event) => {
					if (event.key === "Escape") setDraft(null);
				}}
				aria-label={rename}
				className="h-8 min-w-0 flex-1 rounded-lg text-sm font-semibold"
			/>
			<Button
				type="submit"
				variant="ghost"
				size="icon"
				className="size-8 shrink-0 rounded-lg"
				aria-label={t("memory.entities.saveName", "Save name")}
			>
				<CheckIcon className="size-4" aria-hidden />
			</Button>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="size-8 shrink-0 rounded-lg text-muted-foreground"
				aria-label={t("memory.entities.cancelRename", "Cancel")}
				onClick={() => setDraft(null)}
			>
				<XIcon className="size-4" aria-hidden />
			</Button>
		</form>
	);
}

function MergeButton({
	targets,
	disabled,
	name,
	onMerge,
}: Readonly<{
	targets: BotEntity[];
	disabled: boolean;
	name: string;
	onMerge: (into: BotEntity) => void;
}>) {
	const { t } = useTranslation("bots");
	const [open, setOpen] = useState(false);
	const you = t("memory.entities.you", "You");
	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="rounded-full"
					disabled={disabled || targets.length === 0}
				>
					<GitMergeIcon className="size-3.5" aria-hidden />
					{t("memory.entities.merge", "Merge into…")}
				</Button>
			</PopoverTrigger>
			<PopoverContent align="start" className="w-64 p-0">
				<Command>
					<CommandInput
						placeholder={t(
							"memory.entities.mergeSearch",
							"Find who {{name}} is the same as…",
							{ name },
						)}
					/>
					<CommandList>
						<CommandEmpty>
							{t("memory.entities.mergeEmpty", "Nobody found")}
						</CommandEmpty>
						<CommandGroup>
							{targets.map((target) => {
								const Icon = entityStyle(target).icon;
								const label = entityLabel(target, you);
								return (
									<CommandItem
										key={entityKey(target)}
										value={[label, ...target.aliases, target.id].join(" ")}
										onSelect={() => {
											setOpen(false);
											onMerge(target);
										}}
									>
										<Icon
											className="size-4 text-muted-foreground"
											aria-hidden
										/>
										<span className="min-w-0 flex-1 truncate">{label}</span>
										<span className="text-[11px] tabular-nums text-muted-foreground">
											{target.fact_count}
										</span>
									</CommandItem>
								);
							})}
						</CommandGroup>
					</CommandList>
				</Command>
			</PopoverContent>
		</Popover>
	);
}

function ForgetEntityButton({
	name,
	count,
	disabled,
	onConfirm,
}: Readonly<{
	name: string;
	count: number;
	disabled: boolean;
	onConfirm: () => void;
}>) {
	const { t } = useTranslation("bots");
	const [open, setOpen] = useState(false);
	return (
		<>
			<Button
				type="button"
				variant="outline"
				size="sm"
				className="rounded-full text-destructive hover:text-destructive"
				disabled={disabled}
				onClick={() => setOpen(true)}
			>
				<Trash2Icon className="size-3.5" aria-hidden />
				{t("memory.entities.forget", "Forget everything about {{name}}", {
					name,
				})}
			</Button>
			<AlertDialog open={open} onOpenChange={setOpen}>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("memory.entities.forgetTitle", "Forget {{name}}?", { name })}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"memory.entities.forgetBody",
								"{{count}} facts that mention {{name}} are deleted. The chat keeps what was said.",
								{
									count,
									name,
									defaultValue_one:
										"{{count}} fact that mentions {{name}} is deleted. The chat keeps what was said.",
								},
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{t("memory.entities.cancelRename", "Cancel")}
						</AlertDialogCancel>
						<AlertDialogAction
							className="bg-destructive text-white hover:bg-destructive/90"
							onClick={onConfirm}
						>
							{t("memory.entities.forgetConfirm", "Forget")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}

export function BotEntityDetail({
	owner,
	entity,
	entities,
	facts,
	actions,
	onSelect,
	onBack,
	onChanged,
}: Readonly<{
	owner: BotFactsOwner;
	entity: BotEntity;
	entities: BotEntity[];
	facts: BotFact[];
	actions: EntityFactActions;
	onSelect: (entity: BotEntity) => void;
	onBack: () => void;
	onChanged: (selectKey?: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const kindName = useEntityKindName();
	const you = t("memory.entities.you", "You");
	const label = entityLabel(entity, you);
	const style = entityStyle(entity);
	const Icon = style.icon;
	const ref = { id: entity.id, scope: entity.conversation_scope };
	const related = useMemo(() => entityFacts(facts, entity), [facts, entity]);
	const graph = useMemo(
		() => egoGraph(entity, entities, facts),
		[entity, entities, facts],
	);
	const targets = useMemo(
		() => mergeTargets(entities, entity),
		[entities, entity],
	);
	const aliases = entityAliases(entity, you);
	const failed = (error: unknown) => toast.error(getErrorMessage(error));
	const update = useMutation({
		mutationFn: (change: { name?: string; kind?: string }) =>
			updateBotEntity(owner, ref, change),
		onSuccess: () => onChanged(),
		onError: failed,
	});
	const merge = useMutation({
		mutationFn: (into: BotEntity) => mergeBotEntity(owner, ref, into.id),
		onSuccess: (merged) => onChanged(entityKey(merged)),
		onError: failed,
	});
	const forget = useMutation({
		mutationFn: () => forgetBotEntity(owner, ref),
		onSuccess: () => onChanged(""),
		onError: failed,
	});
	const busy = update.isPending || merge.isPending || forget.isPending;

	return (
		<section
			aria-label={label}
			className="min-w-0 space-y-4 rounded-2xl border border-border/65 bg-card p-4 shadow-xs"
		>
			<div className="flex items-start gap-3">
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="-ml-1 size-8 shrink-0 rounded-lg @2xl/entities:hidden"
					aria-label={t("memory.entities.back", "All people & things")}
					onClick={onBack}
				>
					<ArrowLeftIcon className="size-4" aria-hidden />
				</Button>
				<span
					className={cn(
						"flex size-10 shrink-0 items-center justify-center rounded-2xl",
						style.color,
					)}
				>
					<Icon className="size-5" strokeWidth={1.6} aria-hidden />
				</span>
				<div className="min-w-0 flex-1 space-y-1.5">
					<RenameField
						key={entityKey(entity)}
						entity={entity}
						label={label}
						busy={busy}
						onSave={(name) => update.mutate({ name })}
					/>
					<div className="flex flex-wrap items-center gap-1.5">
						{entity.owner ? (
							<span className="text-[11px] text-muted-foreground">
								{t(
									"memory.entities.ownerHint",
									"The person this bot works for",
								)}
							</span>
						) : (
							<Select
								value={entity.kind}
								disabled={busy}
								onValueChange={(kind) => update.mutate({ kind })}
							>
								<SelectTrigger
									size="sm"
									aria-label={t("memory.entities.kindLabel", "Kind")}
									className="h-7 w-auto gap-1.5 rounded-full px-2.5 text-[11px]"
								>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{ENTITY_KINDS.map((kind) => (
										<SelectItem key={kind} value={kind}>
											{kindName(kind)}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						)}
						{entity.conversation_scope && (
							<span className="inline-flex items-center gap-1 rounded-full border border-border/60 px-2 py-0.5 text-[11px] text-muted-foreground">
								<UsersIcon className="size-3" aria-hidden />
								{t("memory.entities.room", "Group chat")}
							</span>
						)}
					</div>
				</div>
			</div>
			{aliases.length > 0 && (
				<ul
					aria-label={t("memory.entities.aliases", "Also called")}
					className="flex flex-wrap gap-1.5"
				>
					{aliases.map((alias) => (
						<li
							key={alias}
							className="max-w-full truncate rounded-full bg-muted px-2.5 py-1 text-[11px] text-muted-foreground"
						>
							{alias}
						</li>
					))}
				</ul>
			)}
			{!entity.owner && (
				<div className="flex flex-wrap gap-2">
					<MergeButton
						targets={targets}
						disabled={busy}
						name={label}
						onMerge={(into) => merge.mutate(into)}
					/>
					<ForgetEntityButton
						name={label}
						count={entity.fact_count}
						disabled={busy}
						onConfirm={() => forget.mutate()}
					/>
				</div>
			)}
			{graph.neighbours.length > 0 && (
				<BotEntityGraph center={entity} graph={graph} onSelect={onSelect} />
			)}
			<div className="space-y-3">
				<h5 className="text-xs font-semibold text-muted-foreground">
					{t("memory.entities.facts", "{{count}} facts", {
						count: related.length,
						defaultValue_one: "{{count}} fact",
					})}
				</h5>
				{related.length === 0 ? (
					<p className="rounded-xl border border-dashed border-border px-4 py-5 text-center text-xs text-muted-foreground">
						{t("memory.entities.noFacts", "No facts about {{name}} yet.", {
							name: label,
						})}
					</p>
				) : (
					<ul className="flex flex-col gap-3">
						{related.map((fact) => (
							<BotMemoryCard
								key={fact.id}
								fact={fact}
								now={actions.now}
								format={actions.format}
								busy={actions.pendingId === fact.id}
								disabled={actions.disabled}
								onForget={actions.onForget}
								onJump={actions.onJump}
							/>
						))}
					</ul>
				)}
			</div>
		</section>
	);
}
