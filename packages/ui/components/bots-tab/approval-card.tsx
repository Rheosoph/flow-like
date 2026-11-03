"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BanIcon,
	CheckCheckIcon,
	CheckIcon,
	ShieldQuestionIcon,
} from "lucide-react";
import { useState } from "react";
import {
	useAnswerBotApproval,
	useBot,
	useBotLabels,
} from "../../hooks/use-bots";
import { cn } from "../../lib/utils";
import type { IMessageApproval } from "../interfaces/chat-default/chat-db";
import { Button } from "../ui/button";
import type { ApprovalDecision } from "./approval-entry";
import type { ConversationApproval } from "./fold-entries";
import { useGrantLabel } from "./grant-labels";

function lowerFirst(summary: string): string {
	const text = summary.trim().replace(/[.!]+$/, "");
	return /^[A-Z][a-z]/.test(text)
		? text.charAt(0).toLowerCase() + text.slice(1)
		: text;
}

function useDecisionText() {
	const { t } = useTranslation("bots");
	return (decision: ApprovalDecision): string =>
		({
			once: t("crew.approval.answeredOnce", "Allowed once"),
			always: t("crew.approval.answeredAlways", "Always allowed"),
			deny: t("crew.approval.answeredDeny", "Denied"),
		})[decision];
}

/** A bot's request to run a call, with Allow once / Always allow / Deny, or the owner's answer. */
export function ApprovalCard({
	approval,
	botId,
	botName,
	readOnly,
}: Readonly<{
	approval: ConversationApproval;
	botId: string;
	botName: string;
	readOnly: boolean;
}>) {
	const { t } = useTranslation("bots");
	const grantLabel = useGrantLabel();
	const decisionText = useDecisionText();
	const { mutate, isPending, isError, variables } = useAnswerBotApproval();
	const [sent, setSent] = useState<ApprovalDecision>();
	const decision = approval.answer ?? sent;
	const rememberable = approval.scopes.length > 0;

	const answer = (next: ApprovalDecision) =>
		mutate(
			{
				botId,
				turnId: approval.turnId,
				approvalId: approval.id,
				decision: next,
			},
			{ onSuccess: () => setSent(next) },
		);

	return (
		<section
			data-slot="approval-card"
			aria-label={t("crew.approval.label", "Approval request")}
			className={cn(
				"flex w-full max-w-prose flex-col gap-2.5 rounded-2xl border bg-card p-3.5",
				!decision && "border-primary/40",
			)}
		>
			<div className="flex items-start gap-2.5">
				<span className="grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
					<ShieldQuestionIcon className="size-4" aria-hidden />
				</span>
				<div className="min-w-0 flex-1">
					<p className="text-sm font-semibold leading-snug wrap-break-word">
						{t("crew.approval.title", "{{name}} wants to {{summary}}", {
							name: botName,
							summary: lowerFirst(approval.summary || approval.tool),
						})}
					</p>
					<p className="mt-0.5 text-xs text-muted-foreground">
						<code className="rounded bg-muted px-1 py-0.5">
							{approval.tool}
						</code>
						{rememberable
							? ` · ${approval.scopes
									.map((scope) => grantLabel(approval.tool, scope))
									.join(", ")}`
							: ` · ${t("crew.approval.askEveryTime", "Always asks first")}`}
					</p>
					{approval.requesterName && (
						<p
							data-slot="approval-requester"
							className="mt-0.5 text-xs text-muted-foreground"
						>
							{approval.room
								? t(
										"crew.approval.triggeredByInRoom",
										"Triggered by {{name}} in {{room}}",
										{ name: approval.requesterName, room: approval.room },
									)
								: t("crew.approval.triggeredBy", "Triggered by {{name}}", {
										name: approval.requesterName,
									})}
						</p>
					)}
				</div>
			</div>
			{decision ? (
				<p
					data-slot="approval-answer"
					className="flex items-center gap-1.5 text-sm text-muted-foreground"
				>
					{decision === "deny" ? (
						<BanIcon className="size-4" aria-hidden />
					) : (
						<CheckIcon className="size-4" aria-hidden />
					)}
					{decisionText(decision)}
				</p>
			) : (
				<>
					<div className="flex flex-wrap gap-2">
						<Button
							type="button"
							size="sm"
							variant="outline"
							disabled={readOnly || isPending}
							onClick={() => answer("once")}
						>
							<CheckIcon aria-hidden />
							{t("crew.approval.once", "Allow once")}
						</Button>
						{rememberable && (
							<Button
								type="button"
								size="sm"
								disabled={readOnly || isPending}
								onClick={() => answer("always")}
							>
								<CheckCheckIcon aria-hidden />
								{t("crew.approval.always", "Always allow")}
							</Button>
						)}
						<Button
							type="button"
							size="sm"
							variant="ghost"
							disabled={readOnly || isPending}
							onClick={() => answer("deny")}
						>
							<BanIcon aria-hidden />
							{t("crew.approval.deny", "Deny")}
						</Button>
					</div>
					{isError && !isPending && variables && (
						<p role="alert" className="text-xs text-destructive">
							{t(
								"crew.approval.failed",
								"Couldn't send your answer. Try again.",
							)}
						</p>
					)}
				</>
			)}
		</section>
	);
}

/** The card as a bot's chat shows it, named after the bot that asks. */
export function ChatApprovalCard({
	approval,
}: Readonly<{ approval: IMessageApproval }>) {
	const bot = useBot(approval.botId).data;
	const labels = useBotLabels();
	const { t } = useTranslation("bots");
	return (
		<ApprovalCard
			approval={approval}
			botId={approval.botId}
			botName={
				(bot && labels(bot).name) || t("crew.approval.botFallback", "Your bot")
			}
			readOnly={false}
		/>
	);
}
