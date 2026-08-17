"use client";

import { useTranslation } from "@flow-like/locales";
import { motion } from "framer-motion";
import {
	ChevronDownIcon,
	LayersIcon,
	ShieldQuestionIcon,
	SparklesIcon,
} from "lucide-react";
import { useState } from "react";
import { Button, Checkbox, cn, useBackend, useInvoke } from "../../index";
import {
	type AskUserForm,
	askUserAnswerPayload,
	initialAskUserDrafts,
	isAskUserFormComplete,
} from "../../lib/ask-user";
import type { GlobalToolPrompt } from "../../state/global-chat/global-chat-store";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../ui/collapsible";
import { AskUserQuestions } from "./ask-user-questions";

const EMPTY_ASK: AskUserForm = { questions: [], batched: false };

/**
 * Resolves an approval's target app id to its name + icon so the card shows the app's identity
 * instead of the opaque id.
 */
function ApprovalAppIdentity({
	appId,
	description,
}: {
	appId: string;
	description?: string;
}) {
	const backend = useBackend();
	const meta = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId],
		!!appId,
	);
	const name = meta.data?.name || appId;
	const icon = meta.data?.icon ?? meta.data?.thumbnail;
	const resolvedDescription = description?.replace(appId, name);

	return (
		<div className="space-y-2">
			<div className="flex items-center gap-2 rounded-lg border border-border/60 bg-muted/40 px-2.5 py-1.5">
				{icon ? (
					<img
						src={icon}
						alt=""
						className="size-5 rounded-sm object-cover shrink-0"
					/>
				) : (
					<span className="flex items-center justify-center size-5 rounded-sm bg-primary/15 text-primary shrink-0">
						<LayersIcon className="size-3" />
					</span>
				)}
				<span className="min-w-0 flex-1 truncate text-[13px] font-medium">
					{name}
				</span>
			</div>
			{resolvedDescription && (
				<ApprovalDetails description={resolvedDescription} />
			)}
		</div>
	);
}

/** Keep verbose delegated prompts out of the way until the user explicitly asks to inspect them. */
function ApprovalDetails({ description }: { description: string }) {
	const { t } = useTranslation("chat");
	const [open, setOpen] = useState(false);

	return (
		<Collapsible open={open} onOpenChange={setOpen}>
			<CollapsibleTrigger asChild>
				<button
					type="button"
					className="flex w-full items-center gap-2 rounded-md px-1 py-1 text-left text-xs font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
				>
					<span>{t("reviewRequestDetails", "Review request details")}</span>
					<ChevronDownIcon
						className={cn(
							"ml-auto size-3.5 shrink-0 transition-transform",
							open && "rotate-180",
						)}
					/>
				</button>
			</CollapsibleTrigger>
			<CollapsibleContent>
				<div className="mt-1 max-h-56 overflow-y-auto overscroll-contain rounded-lg border border-border/50 bg-muted/20 p-2.5">
					<p className="whitespace-pre-wrap break-words text-xs leading-relaxed text-muted-foreground">
						{description}
					</p>
				</div>
			</CollapsibleContent>
		</Collapsible>
	);
}

/**
 * Inline replacement for the former modal tool dialogs: renders the pending approval or question
 * from the global assistant directly in the chat surface, pinned above the input. A question is
 * either a single freeform/choice prompt or the batched BUILD intake form, whose gaps are all
 * answered in this one card before the build starts.
 */
export function InlineToolPrompt({ prompt }: { prompt: GlobalToolPrompt }) {
	const { t } = useTranslation("chat");
	const [remember, setRemember] = useState(false);
	const ask = prompt.ask ?? EMPTY_ASK;
	const [drafts, setDrafts] = useState(() => initialAskUserDrafts(ask));
	const isApproval = prompt.kind === "approval";

	const canSend = isAskUserFormComplete(ask, drafts);
	const isSingleFreeform =
		ask.questions.length === 1 && ask.questions[0].mode === "freeform";

	const submit = () => {
		if (!canSend) return;
		prompt.respond({ answer: askUserAnswerPayload(ask, drafts) });
	};

	return (
		<motion.div
			initial={{ opacity: 0, y: 8, scale: 0.98 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			transition={{ type: "spring", stiffness: 380, damping: 32 }}
			className="flex max-h-[70vh] w-full flex-col overflow-hidden rounded-2xl border bg-card shadow-floating"
			style={{ borderColor: "var(--fl-chat-rule-strong, var(--border))" }}
		>
			<div className="flex shrink-0 items-start gap-3 px-4 pt-4 pb-2">
				<span
					className={cn(
						"flex size-8 shrink-0 items-center justify-center rounded-full",
						isApproval
							? "bg-amber-500/15 text-amber-600 dark:text-amber-400"
							: "bg-primary/15 text-primary",
					)}
				>
					{isApproval ? (
						<ShieldQuestionIcon className="size-4" />
					) : (
						<SparklesIcon className="size-4" />
					)}
				</span>
				<div className="min-w-0 flex-1">
					<p
						className="text-[15px] leading-snug"
						style={{ fontFamily: "var(--fl-chat-prose-font)" }}
					>
						{prompt.title}
					</p>
					<p className="mt-0.5 text-[11px] font-semibold uppercase tracking-widest text-muted-foreground/70">
						{isApproval
							? t("needsYourApproval", "Needs your approval")
							: t("needsAnAnswer", "Needs an answer")}
					</p>
				</div>
			</div>
			<div className="min-h-0 space-y-2.5 overflow-y-auto px-4 pt-1 pb-3">
				{prompt.appId ? (
					<ApprovalAppIdentity
						appId={prompt.appId}
						description={isApproval ? prompt.description : undefined}
					/>
				) : (
					prompt.kind === "ask" &&
					prompt.description && (
						<p className="text-xs text-muted-foreground">
							{prompt.description}
						</p>
					)
				)}
				{isApproval && prompt.description && !prompt.appId && (
					<ApprovalDetails description={prompt.description} />
				)}
				{prompt.kind === "ask" && (
					<AskUserQuestions
						form={ask}
						drafts={drafts}
						onDraftsChange={setDrafts}
						onSubmit={isSingleFreeform ? submit : undefined}
					/>
				)}
			</div>
			<div
				className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-t px-4 py-3"
				style={{ borderColor: "var(--fl-chat-rule, var(--border))" }}
			>
				{isApproval ? (
					<label
						htmlFor="inline-tool-prompt-remember"
						className="flex cursor-pointer items-center gap-2 text-xs text-muted-foreground"
					>
						<Checkbox
							id="inline-tool-prompt-remember"
							checked={remember}
							onCheckedChange={(checked) => setRemember(checked === true)}
						/>
						{t("donapostAskAgainThisSession", "Don't ask again this session")}
					</label>
				) : (
					<span />
				)}
				<div className="ml-auto flex items-center gap-2">
					<Button
						variant="outline"
						size="sm"
						className="rounded-full outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
						onClick={() =>
							isApproval
								? prompt.respond({ approved: false, remember: false })
								: prompt.respond(null)
						}
					>
						{isApproval ? "Deny" : "Cancel"}
					</Button>
					<Button
						size="sm"
						disabled={!isApproval && !canSend}
						className="rounded-full outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0"
						onClick={() => {
							if (isApproval) {
								prompt.respond({ approved: true, remember });
							} else {
								submit();
							}
						}}
					>
						{isApproval ? "Approve" : "Send"}
					</Button>
				</div>
			</div>
		</motion.div>
	);
}
