"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronLeft, ChevronRight, Copy, Trash2 } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useBotLabels } from "../../../hooks/use-bots";
import { Button } from "../../ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import { useBotProblemMessage } from "./bot-editor-copy";
import { getBotProblemStep } from "./bot-editor-steps";
import type { BotEditor } from "./use-bot-editor";
import type { BotEditorSubmit } from "./use-bot-editor-submit";

export interface BotDiscardPrompt {
	requested: boolean;
	keep: () => void;
	discard: () => void;
}

function Confirm({
	message,
	cancelLabel,
	confirmLabel,
	destructive = false,
	busy = false,
	onCancel,
	onConfirm,
}: Readonly<{
	message: string;
	cancelLabel: string;
	confirmLabel: string;
	destructive?: boolean;
	busy?: boolean;
	onCancel: () => void;
	onConfirm: () => void;
}>) {
	return (
		<div role="alert" className="flex flex-wrap items-center gap-2">
			<span className="text-sm">{message}</span>
			<Button size="sm" variant="ghost" onClick={onCancel}>
				{cancelLabel}
			</Button>
			<Button
				size="sm"
				variant={destructive ? "destructive" : "default"}
				disabled={busy}
				onClick={onConfirm}
			>
				{confirmLabel}
			</Button>
		</div>
	);
}

function FooterStatus({
	editor,
	submit,
	discard,
	showProblems,
}: Readonly<{
	editor: BotEditor;
	submit: BotEditorSubmit;
	discard: BotDiscardPrompt;
	showProblems: boolean;
}>): ReactNode {
	const { t } = useTranslation("bots");
	const problemMessage = useBotProblemMessage();
	const labels = useBotLabels();
	const [confirmingDelete, setConfirmingDelete] = useState(false);
	const bot = editor.original;
	if (discard.requested)
		return (
			<Confirm
				message={t("editor.discard.prompt")}
				cancelLabel={t("editor.discard.keep")}
				confirmLabel={t("editor.discard.confirm")}
				destructive
				onCancel={discard.keep}
				onConfirm={discard.discard}
			/>
		);
	if (confirmingDelete && bot)
		return (
			<Confirm
				message={t("editor.delete.confirm", { name: labels(bot).name })}
				cancelLabel={t("editor.actions.cancel")}
				confirmLabel={
					submit.deleting
						? t("editor.actions.deleting")
						: t("editor.delete.action")
				}
				destructive
				busy={submit.deleting}
				onCancel={() => setConfirmingDelete(false)}
				onConfirm={() => void submit.remove()}
			/>
		);
	if (submit.error)
		return (
			<p role="alert" className="text-sm text-destructive">
				{submit.error}
			</p>
		);
	const problem = showProblems ? editor.problems[0] : undefined;
	if (problem) {
		const step = getBotProblemStep(problem);
		return (
			<p className="flex flex-wrap items-center gap-x-2 text-sm text-muted-foreground">
				{problemMessage(problem)}
				{step && step !== editor.step && (
					<Button
						variant="link"
						size="sm"
						className="h-auto px-0"
						onClick={() => editor.goTo(step)}
					>
						{t("editor.problems.fix")}
					</Button>
				)}
			</p>
		);
	}
	if (!bot) return null;
	const busy = submit.saving || submit.deleting || submit.duplicating;
	return (
		<div className="flex flex-wrap items-center gap-1">
			{submit.duplicate && (
				<DuplicateButton
					duplicate={submit.duplicate}
					duplicating={submit.duplicating}
					disabled={busy}
					unsaved={editor.dirty}
				/>
			)}
			{!bot.system && (
				<Button
					variant="ghost"
					size="sm"
					className="text-destructive hover:text-destructive"
					disabled={busy}
					onClick={() => setConfirmingDelete(true)}
				>
					<Trash2 />
					{t("editor.actions.delete")}
				</Button>
			)}
		</div>
	);
}

/**
 * Copies the bot as saved. Unsaved changes hold it back; it stays focusable then (`aria-disabled`)
 * so its tooltip can say why, and the duplicate itself refuses to start.
 */
function DuplicateButton({
	duplicate,
	duplicating,
	disabled,
	unsaved,
}: Readonly<{
	duplicate: () => Promise<void>;
	duplicating: boolean;
	disabled: boolean;
	unsaved: boolean;
}>) {
	const { t } = useTranslation("bots");
	const button = (
		<Button
			variant="ghost"
			size="sm"
			disabled={disabled}
			aria-disabled={unsaved || undefined}
			className="aria-disabled:cursor-not-allowed aria-disabled:opacity-50"
			onClick={() => void duplicate()}
		>
			<Copy />
			{duplicating
				? t("editor.actions.duplicating")
				: t("editor.actions.duplicate")}
		</Button>
	);
	if (!unsaved) return button;
	return (
		<Tooltip>
			<TooltipTrigger asChild>{button}</TooltipTrigger>
			<TooltipContent>{t("editor.duplicate.unsaved")}</TooltipContent>
		</Tooltip>
	);
}

export function BotEditorFooter({
	editor,
	submit,
	discard,
}: Readonly<{
	editor: BotEditor;
	submit: BotEditorSubmit;
	discard: BotDiscardPrompt;
}>) {
	const { t } = useTranslation("bots");
	const index = editor.steps.indexOf(editor.step);
	const isFirst = index <= 0;
	const isLast = index === editor.steps.length - 1;
	const creating = editor.mode === "create";
	const showPrimary = !creating || isLast;
	const blocked = editor.problems.length > 0;
	const primaryDisabled =
		blocked || submit.saving || submit.deleting || (!creating && !editor.dirty);
	return (
		<footer className="flex shrink-0 flex-col gap-3 border-t border-border px-4 py-3 sm:flex-row sm:items-center sm:px-6">
			<div className="flex min-h-9 min-w-0 flex-1 items-center">
				<FooterStatus
					editor={editor}
					submit={submit}
					discard={discard}
					showProblems={showPrimary && (creating || editor.dirty)}
				/>
			</div>
			<div className="flex items-center justify-end gap-2">
				<Button
					variant="ghost"
					disabled={isFirst}
					onClick={() => editor.goTo(editor.steps[index - 1])}
				>
					<ChevronLeft />
					{t("editor.actions.back")}
				</Button>
				{!isLast && (
					<Button
						variant={creating ? "default" : "outline"}
						onClick={() => editor.goTo(editor.steps[index + 1])}
					>
						{t("editor.actions.next")}
						<ChevronRight />
					</Button>
				)}
				{showPrimary && (
					<Button
						disabled={primaryDisabled}
						onClick={() => void submit.submit()}
					>
						{creating
							? submit.saving
								? t("editor.actions.creating")
								: t("editor.actions.create")
							: submit.saving
								? t("editor.actions.saving")
								: t("editor.actions.save")}
					</Button>
				)}
			</div>
		</footer>
	);
}
