"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CheckIcon,
	Loader2Icon,
	PauseIcon,
	PencilIcon,
	PlusIcon,
	Trash2Icon,
	UploadIcon,
} from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useBotSkillAction, useBotSkills } from "../../hooks/use-bots";
import { getErrorMessage } from "../../lib/error-message";
import type { IBotSkill } from "../../state/backend-state/bot-state";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Textarea } from "../ui/textarea";
import {
	SKILL_ZIP_MAX_BYTES,
	type SkillDraft,
	canApprove,
	canEdit,
	draftProblem,
	sortSkills,
	sourceKind,
} from "./bot-skill-model";

const EMPTY_DRAFT: SkillDraft = { name: "", description: "", body: "" };

function useSourceLabel() {
	const { t } = useTranslation("bots");
	return (skill: IBotSkill): string => {
		switch (sourceKind(skill.source)) {
			case "package":
				return t("panel.skills.sourcePackage", "Package");
			case "bot":
				return t("panel.skills.sourceBot", "Proposed by the bot");
			case "import":
				return t("panel.skills.sourceImport", "Imported");
			default:
				return t("panel.skills.sourceOwner", "Written by you");
		}
	};
}

function SkillForm({
	initial,
	editing,
	busy,
	onSave,
	onCancel,
}: Readonly<{
	initial: SkillDraft;
	editing: boolean;
	busy: boolean;
	onSave: (draft: SkillDraft) => void;
	onCancel: () => void;
}>) {
	const { t } = useTranslation("bots");
	const [draft, setDraft] = useState(initial);
	const problem = draftProblem(draft, editing);
	const complete = Boolean(
		draft.name.trim() && draft.description.trim() && draft.body.trim(),
	);
	const messages = {
		name: t(
			"panel.skills.problemName",
			"Use lowercase letters, digits and single hyphens, at most 64 characters.",
		),
		description: t(
			"panel.skills.problemDescription",
			"Describe the skill in one line.",
		),
		body: t("panel.skills.problemBody", "The instructions are missing."),
		length: t(
			"panel.skills.problemLength",
			"The description is limited to 1,024 characters and the instructions to 32 KiB.",
		),
	} as const;
	return (
		<form
			data-slot="bot-skill-form"
			className="flex flex-col gap-2 rounded-lg border bg-muted/40 p-3"
			onSubmit={(event) => {
				event.preventDefault();
				if (!problem && complete) onSave(draft);
			}}
		>
			<Input
				aria-label={t("panel.skills.nameLabel", "Skill name")}
				placeholder={t("panel.skills.name", "Name, e.g. weekly-report")}
				value={draft.name}
				disabled={editing}
				className="h-8 font-mono text-xs"
				onChange={(event) => setDraft({ ...draft, name: event.target.value })}
			/>
			<Input
				aria-label={t("panel.skills.descriptionLabel", "Description")}
				placeholder={t(
					"panel.skills.description",
					"What it does and when to use it",
				)}
				value={draft.description}
				className="h-8 text-xs"
				onChange={(event) =>
					setDraft({ ...draft, description: event.target.value })
				}
			/>
			<Textarea
				aria-label={t("panel.skills.bodyLabel", "Instructions")}
				placeholder={t(
					"panel.skills.body",
					"The steps the bot follows, in markdown",
				)}
				value={draft.body}
				rows={8}
				className="text-xs"
				onChange={(event) => setDraft({ ...draft, body: event.target.value })}
			/>
			{problem && complete && (
				<p role="alert" className="text-xs text-destructive">
					{messages[problem]}
				</p>
			)}
			<div className="flex justify-end gap-2">
				<Button type="button" variant="ghost" size="sm" onClick={onCancel}>
					{t("panel.skills.cancel", "Cancel")}
				</Button>
				<Button
					type="submit"
					size="sm"
					disabled={busy || !complete || !!problem}
				>
					{busy && <Loader2Icon className="animate-spin" aria-hidden />}
					{t("panel.skills.save", "Save skill")}
				</Button>
			</div>
		</form>
	);
}

function SkillRow({
	skill,
	busy,
	onApprove,
	onDisable,
	onEdit,
	onDelete,
}: Readonly<{
	skill: IBotSkill;
	busy: boolean;
	onApprove: () => void;
	onDisable: () => void;
	onEdit: () => void;
	onDelete: () => void;
}>) {
	const { t } = useTranslation("bots");
	const sourceLabel = useSourceLabel();
	const statusLabel = {
		active: t("panel.skills.statusActive", "Active"),
		pending: t("panel.skills.statusPending", "Waiting for you"),
		disabled: t("panel.skills.statusDisabled", "Off"),
	}[skill.status];
	return (
		<li
			data-slot="bot-skill"
			data-status={skill.status}
			className="flex min-w-0 flex-col gap-1.5 rounded-lg border p-2.5"
		>
			<div className="flex min-w-0 flex-wrap items-center gap-1.5">
				<span className="truncate font-mono text-xs font-medium">
					{skill.name}
				</span>
				<Badge variant={skill.status === "pending" ? "default" : "secondary"}>
					{statusLabel}
				</Badge>
				<span className="text-xs text-muted-foreground">
					{sourceLabel(skill)}
				</span>
			</div>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{skill.description}
			</p>
			{skill.status === "pending" && (
				<p className="text-xs text-muted-foreground">
					{t(
						"panel.skills.pendingNote",
						"The bot can't see this skill until you approve it.",
					)}
				</p>
			)}
			<div className="flex flex-wrap justify-end gap-1">
				{canApprove(skill) && (
					<Button type="button" size="sm" disabled={busy} onClick={onApprove}>
						<CheckIcon aria-hidden />
						{skill.status === "pending"
							? t("panel.skills.approve", "Approve")
							: t("panel.skills.enable", "Switch on")}
					</Button>
				)}
				{skill.status === "active" && (
					<Button
						type="button"
						variant="outline"
						size="sm"
						disabled={busy}
						onClick={onDisable}
					>
						<PauseIcon aria-hidden />
						{t("panel.skills.disable", "Switch off")}
					</Button>
				)}
				{canEdit(skill) && (
					<Button
						type="button"
						variant="ghost"
						size="icon"
						className="size-8"
						aria-label={t("panel.skills.edit", "Edit {{name}}", {
							name: skill.name,
						})}
						onClick={onEdit}
					>
						<PencilIcon aria-hidden />
					</Button>
				)}
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="size-8"
					aria-label={t("panel.skills.delete", "Delete {{name}}", {
						name: skill.name,
					})}
					disabled={busy}
					onClick={onDelete}
				>
					<Trash2Icon aria-hidden />
				</Button>
			</div>
		</li>
	);
}

/** The Skills card of the bot editor: what the bot follows, what waits for you, import. */
export function BotSkillsCard({ botId }: Readonly<{ botId: string }>) {
	const { t } = useTranslation("bots");
	const skills = useBotSkills(botId);
	const act = useBotSkillAction();
	const fileInput = useRef<HTMLInputElement>(null);
	const [form, setForm] = useState<{ skill: IBotSkill | null } | null>(null);
	const sorted = useMemo(() => sortSkills(skills.data ?? []), [skills.data]);

	const run = (
		action: Parameters<typeof act.mutate>[0]["action"],
		done?: () => void,
	) =>
		act.mutate(
			{ botId, action },
			{
				onSuccess: done,
				onError: (error) => toast.error(getErrorMessage(error)),
			},
		);

	const importZip = async (file: File | undefined) => {
		if (!file) return;
		if (file.size > SKILL_ZIP_MAX_BYTES) {
			toast.error(
				t("panel.skills.zipTooLarge", "A skill zip is at most 8 MiB."),
			);
			return;
		}
		const zip = new Uint8Array(await file.arrayBuffer());
		run({ type: "import", zip }, () =>
			toast.success(
				t(
					"panel.skills.imported",
					"Imported. Review the skill and approve it to use it.",
				),
			),
		);
	};

	return (
		<div data-slot="bot-skills" className="flex min-w-0 flex-col gap-2">
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t(
					"panel.skills.hint",
					"Skills are instructions the bot loads before a task they describe. Only skills you approved reach the bot.",
				)}
			</p>
			{skills.isLoading && (
				<Loader2Icon className="size-4 animate-spin" aria-hidden />
			)}
			{!skills.isLoading && sorted.length === 0 && !form && (
				<p className="text-xs text-muted-foreground">
					{t("panel.skills.none", "No skills yet.")}
				</p>
			)}
			<ul className="flex flex-col gap-1.5">
				{sorted.map((skill) =>
					form?.skill?.id === skill.id ? (
						<li key={skill.id}>
							<SkillForm
								initial={{
									name: skill.name,
									description: skill.description,
									body: skill.body,
								}}
								editing
								busy={act.isPending}
								onCancel={() => setForm(null)}
								onSave={(draft) =>
									run(
										{
											type: "update",
											skillId: skill.id,
											request: {
												description: draft.description.trim(),
												body: draft.body.trim(),
											},
										},
										() => setForm(null),
									)
								}
							/>
						</li>
					) : (
						<SkillRow
							key={skill.id}
							skill={skill}
							busy={act.isPending}
							onApprove={() => run({ type: "approve", skillId: skill.id })}
							onDisable={() => run({ type: "disable", skillId: skill.id })}
							onEdit={() => setForm({ skill })}
							onDelete={() => run({ type: "delete", skillId: skill.id })}
						/>
					),
				)}
			</ul>
			{form && !form.skill && (
				<SkillForm
					initial={EMPTY_DRAFT}
					editing={false}
					busy={act.isPending}
					onCancel={() => setForm(null)}
					onSave={(draft) =>
						run(
							{
								type: "create",
								request: {
									name: draft.name.trim(),
									description: draft.description.trim(),
									body: draft.body.trim(),
								},
							},
							() => setForm(null),
						)
					}
				/>
			)}
			<div className="flex flex-wrap gap-2">
				<Button
					type="button"
					variant="outline"
					size="sm"
					onClick={() => setForm({ skill: null })}
				>
					<PlusIcon aria-hidden />
					{t("panel.skills.add", "Add a skill")}
				</Button>
				<Button
					type="button"
					variant="outline"
					size="sm"
					disabled={act.isPending}
					onClick={() => fileInput.current?.click()}
				>
					<UploadIcon aria-hidden />
					{t("panel.skills.import", "Import a zip")}
				</Button>
				<input
					ref={fileInput}
					type="file"
					accept=".zip,application/zip"
					className="hidden"
					data-slot="bot-skill-zip"
					onChange={(event) => {
						void importZip(event.target.files?.[0]);
						event.target.value = "";
					}}
				/>
			</div>
		</div>
	);
}
