"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BanIcon,
	CheckCheckIcon,
	CheckIcon,
	KeyRoundIcon,
	ShieldQuestionIcon,
	SlidersHorizontalIcon,
} from "lucide-react";
import { useState } from "react";
import {
	useAnswerBotApproval,
	useBot,
	useBotLabels,
	useUpsertBotSecret,
} from "../../hooks/use-bots";
import { BotAvatar } from "../bots/bot-avatar";
import type { BotAppearance } from "../bots/types";
import type { IMessageApproval } from "../interfaces/chat-default/chat-db";
import {
	RequestCard,
	RequestCardBody,
	RequestCardFooter,
	RequestCardHeader,
	RequestMark,
	type RequestTone,
	requestButtonClass,
} from "../interfaces/chat-default/request-card";
import { Button } from "../ui/button";
import type { ApprovalDecision } from "./approval-entry";
import { BotConfigurationChanges } from "./configuration-changes";
import type { ConversationApproval } from "./fold-entries";
import { useGrantLabel } from "./grant-labels";
import { SecretRequestForm } from "./secret-request-form";

/** The summary as the end of "{{name}} wants to …"; core phrases them as "The bot wants to …". */
function lowerFirst(summary: string): string {
	const text = summary
		.trim()
		.replace(/^the bot wants to\s+/i, "")
		.replace(/[.!]+$/, "");
	return /^[A-Z][a-z]/.test(text)
		? text.charAt(0).toLowerCase() + text.slice(1)
		: text;
}

function useDecisionText() {
	const { t } = useTranslation("bots");
	return (decision: ApprovalDecision, secret?: boolean): string => {
		if (secret)
			return decision === "deny"
				? t("crew.approval.secret.declined", "Declined")
				: t("crew.approval.secret.saved", "Secret saved");
		return {
			once: t("crew.approval.answeredOnce", "Allowed once"),
			always: t("crew.approval.answeredAlways", "Always allowed"),
			deny: t("crew.approval.answeredDeny", "Denied"),
		}[decision];
	};
}

function decisionTone(decision: ApprovalDecision | undefined): RequestTone {
	if (!decision) return "warning";
	return decision === "deny" ? "critical" : "good";
}

function decisionIcon(decision: ApprovalDecision | undefined, secret: boolean) {
	if (!decision) return secret ? KeyRoundIcon : ShieldQuestionIcon;
	return decision === "deny" ? BanIcon : CheckIcon;
}

/** A bot's request to run a call, with Allow once / Always allow / Deny, or the owner's answer. */
export function ApprovalCard({
	approval,
	botId,
	botName,
	botAppearance,
	readOnly,
}: Readonly<{
	approval: ConversationApproval;
	botId: string;
	botName: string;
	/** The asking bot's look; without it the card shows a shield instead of its face. */
	botAppearance?: Partial<BotAppearance>;
	readOnly: boolean;
}>) {
	const { t } = useTranslation("bots");
	const grantLabel = useGrantLabel(approval.scopes);
	const decisionText = useDecisionText();
	const {
		mutate,
		mutateAsync: answerAsync,
		isPending,
		isError,
		variables,
	} = useAnswerBotApproval();
	const upsertSecret = useUpsertBotSecret();
	const [sent, setSent] = useState<ApprovalDecision>();
	const decision = approval.answer ?? sent;
	const isConfiguration = approval.tool === "bot_update_configuration";
	const rememberable = !isConfiguration && approval.scopes.length > 0;
	const tone = decisionTone(decision);
	const disabled = readOnly || isPending;

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

	const secret = approval.secret;
	const icon =
		!decision && isConfiguration
			? SlidersHorizontalIcon
			: decisionIcon(decision, !!secret);

	const saveSecret = async (value: string) => {
		if (!secret) return;
		await upsertSecret.mutateAsync({
			botId,
			name: secret.name,
			request: { value, description: secret.description },
		});
		await answerAsync({
			botId,
			turnId: approval.turnId,
			approvalId: approval.id,
			decision: "once",
		});
		setSent("once");
	};
	return (
		<RequestCard
			data-slot="approval-card"
			aria-label={t("crew.approval.label", "Approval request")}
			tone={tone}
			live={!decision}
			className="max-w-prose"
		>
			<RequestCardHeader
				dense
				tone={tone}
				live={!decision}
				mark={
					<RequestMark icon={icon} tone={tone}>
						{botAppearance && (
							<BotAvatar
								appearance={botAppearance}
								size={36}
								animated={false}
								decorative
							/>
						)}
					</RequestMark>
				}
				eyebrow={
					decision ? (
						<span data-slot="approval-answer">
							{isConfiguration
								? decision === "deny"
									? t(
											"crew.approval.configuration.rejected",
											"Changes rejected",
										)
									: t(
											"crew.approval.configuration.approved",
											"Changes approved",
										)
								: decisionText(decision, !!secret)}
						</span>
					) : (
						t("crew.approval.label", "Approval request")
					)
				}
				className={decision ? "pb-3.5" : undefined}
				title={
					secret
						? t("crew.approval.secret.title", "{{name}} needs a secret", {
								name: botName,
							})
						: isConfiguration
							? t(
									"crew.approval.configuration.requestTitle",
									"{{name}} wants to update its settings",
									{ name: botName },
								)
							: t("crew.approval.title", "{{name}} wants to {{summary}}", {
									name: botName,
									summary: lowerFirst(approval.summary || approval.tool),
								})
				}
				description={
					<>
						{isConfiguration && !decision && (
							<p>
								{t(
									"crew.approval.configuration.hint",
									"Only the settings below will change, starting with the next turn.",
								)}
							</p>
						)}
						{!secret && !isConfiguration && (
							<p className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs">
								<code className="rounded-md border border-border/60 bg-muted/60 px-1.5 py-0.5 font-mono text-[11px] text-foreground/80">
									{approval.tool}
								</code>
								<span>
									{rememberable
										? approval.scopes
												.map((scope) => grantLabel(approval.tool, scope))
												.join(", ")
										: t("crew.approval.askEveryTime", "Always asks first")}
								</span>
							</p>
						)}
						{approval.requesterName && (
							<p data-slot="approval-requester" className="mt-1 text-xs">
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
					</>
				}
			/>
			{approval.configurationChanges && (
				<RequestCardBody className="max-h-[min(32rem,60vh)] overflow-y-auto overscroll-contain px-3.5 py-2.5">
					<BotConfigurationChanges changes={approval.configurationChanges} />
				</RequestCardBody>
			)}
			{!decision && secret && (
				<SecretRequestForm
					secret={secret}
					botName={botName}
					disabled={readOnly || isPending}
					onSave={saveSecret}
					onDecline={() => answer("deny")}
				/>
			)}
			{!decision && !secret && (
				<RequestCardFooter
					className={
						isConfiguration
							? "px-3.5 py-2.5 [&>div:last-child]:max-w-full [&>div:last-child]:flex-wrap [&>div:last-child]:justify-end"
							: "px-3.5 py-2.5"
					}
					start={
						isError &&
						!isPending &&
						variables && (
							<p role="alert" className="text-xs text-destructive">
								{t(
									"crew.approval.failed",
									"Couldn't send your answer. Try again.",
								)}
							</p>
						)
					}
				>
					<Button
						type="button"
						size="sm"
						variant="ghost"
						className={requestButtonClass}
						disabled={disabled}
						onClick={() => answer("deny")}
					>
						<BanIcon aria-hidden />
						{isConfiguration
							? t("crew.approval.configuration.reject", "Reject changes")
							: t("crew.approval.deny", "Deny")}
					</Button>
					<Button
						type="button"
						size="sm"
						variant={rememberable ? "outline" : "default"}
						className={requestButtonClass}
						disabled={disabled}
						onClick={() => answer("once")}
					>
						<CheckIcon aria-hidden />
						{isConfiguration
							? t("crew.approval.configuration.approve", "Approve changes")
							: t("crew.approval.once", "Allow once")}
					</Button>
					{rememberable && (
						<Button
							type="button"
							size="sm"
							className={requestButtonClass}
							disabled={disabled}
							onClick={() => answer("always")}
						>
							<CheckCheckIcon aria-hidden />
							{t("crew.approval.always", "Always allow")}
						</Button>
					)}
				</RequestCardFooter>
			)}
		</RequestCard>
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
			botAppearance={bot?.appearance}
			readOnly={false}
		/>
	);
}
