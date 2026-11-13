"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BanIcon,
	CheckCheckIcon,
	CheckIcon,
	CircleSlashIcon,
	KeyRoundIcon,
	ShieldQuestionIcon,
	SlidersHorizontalIcon,
	TimerOffIcon,
	UserPlusIcon,
} from "lucide-react";
import { useBotSecretSealing } from "../../hooks/use-bot-secret-sealing";
import { useBot, useBotLabels, useUpsertBotSecret } from "../../hooks/use-bots";
import { requiresFreshToolApproval } from "../../lib/frontend-tool-approval-scope";
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
import { type ApprovalClosure, useApprovalAnswer } from "./approval-answer";
import {
	ApprovalBatchCard,
	RequesterLine,
	ToolChip,
	WebContentNotice,
	useDecisionText,
} from "./approval-batch";
import { callPhrase, isBatch } from "./approval-batch-model";
import type { ApprovalDecision } from "./approval-entry";
import {
	BotConfigurationChanges,
	useBotSettingsApprovalText,
} from "./configuration-changes";
import type { ConversationApproval } from "./fold-entries";
import { useGrantLabel } from "./grant-labels";
import { InviteRequestDetails } from "./invite-request";
import { SecretRequestForm } from "./secret-request-form";

const decisionTone = (
	decision: ApprovalDecision | undefined,
	closure: ApprovalClosure | undefined,
): RequestTone => {
	if (closure) return "muted";
	if (!decision) return "warning";
	return decision === "deny" ? "critical" : "good";
};

function decisionIcon(
	decision: ApprovalDecision | undefined,
	closure: ApprovalClosure | undefined,
	secret: boolean,
) {
	if (closure) return closure === "expired" ? TimerOffIcon : CircleSlashIcon;
	if (!decision) return secret ? KeyRoundIcon : ShieldQuestionIcon;
	return decision === "deny" ? BanIcon : CheckIcon;
}

/**
 * Below 24rem the three answers no longer fit one row: the buttons wrap, and Always allow takes the
 * second row. Wider footers are unchanged.
 */
const NARROW_FOOTER =
	"@container @max-sm:[&>div:first-child:empty]:hidden @max-sm:[&>div:last-child]:w-full @max-sm:[&>div:last-child]:flex-wrap @max-sm:[&>div:last-child]:justify-end";

interface ApprovalCardProps {
	approval: ConversationApproval;
	botId: string;
	botName: string;
	/** The asking bot's look; without it the card shows a shield instead of its face. */
	botAppearance?: Partial<BotAppearance>;
	readOnly: boolean;
}

/**
 * A bot's request to run a call, with Allow once / Always allow / Deny, or the owner's answer. A
 * batch of calls from one model round is one card with the same shell.
 */
export function ApprovalCard(props: Readonly<ApprovalCardProps>) {
	return isBatch(props.approval) ? (
		<ApprovalBatchCard {...props} />
	) : (
		<SingleApprovalCard {...props} />
	);
}

function SingleApprovalCard({
	approval,
	botId,
	botName,
	botAppearance,
	readOnly,
}: Readonly<ApprovalCardProps>) {
	const { t } = useTranslation("bots");
	const grantLabel = useGrantLabel(approval.scopes);
	const decisionText = useDecisionText();
	const state = useApprovalAnswer(approval, botId);
	const upsertSecret = useUpsertBotSecret();
	const decision = state.answer;
	const closure = state.closure;
	const waiting = !decision && !closure;
	const isConfiguration = requiresFreshToolApproval(approval.tool);
	const settingsText = useBotSettingsApprovalText(approval.tool);
	const flagged = approval.afterWebContent === true;
	const rememberable =
		!isConfiguration && !flagged && approval.scopes.length > 0;
	const tone = decisionTone(decision, closure);
	const disabled = readOnly || state.pending || state.elsewhere;
	const target = approval.scopes
		.map((scope) => grantLabel(approval.tool, scope))
		.join(", ");

	const answer = (next: ApprovalDecision) => state.send(next);

	const secret = approval.secret;
	const invite = approval.invite;
	const waitingIcon = isConfiguration
		? SlidersHorizontalIcon
		: invite
			? UserPlusIcon
			: undefined;
	const icon =
		waiting && waitingIcon
			? waitingIcon
			: decisionIcon(decision, closure, !!secret);

	const sealing = useBotSecretSealing(secret && waiting ? botId : null);
	const saveSecret = async (value: string, sealed: boolean) => {
		if (!secret) return;
		await upsertSecret.mutateAsync({
			botId,
			name: secret.name,
			request: { value, description: secret.description },
			sealed,
		});
		await state.sendAsync("once");
	};
	const closed = !waiting;
	return (
		<RequestCard
			data-slot="approval-card"
			aria-label={t("crew.approval.label", "Approval request")}
			tone={tone}
			live={waiting}
			className="max-w-prose"
		>
			<RequestCardHeader
				dense
				tone={tone}
				live={waiting}
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
					closure ? (
						<span data-slot="approval-closure">
							{closure === "expired"
								? t("crew.approval.expired", "Expired")
								: t("crew.approval.cancelled", "Task cancelled")}
						</span>
					) : decision ? (
						<span data-slot="approval-answer">
							{isConfiguration
								? decision === "deny"
									? settingsText.rejected
									: settingsText.approved
								: decisionText(decision, !!secret)}
						</span>
					) : (
						t("crew.approval.label", "Approval request")
					)
				}
				className={closed ? "pb-3.5" : undefined}
				title={
					secret
						? t("crew.approval.secret.title", "{{name}} needs a secret", {
								name: botName,
							})
						: isConfiguration
							? settingsText.requestTitle(botName)
							: invite
								? t(
										"crew.approval.invite.title",
										"{{name}} wants to invite {{person}}",
										{ name: botName, person: invite.person },
									)
								: t("crew.approval.title", "{{name}} wants to {{summary}}", {
										name: botName,
										summary: callPhrase(approval.summary || approval.tool),
									})
				}
				description={
					<>
						{isConfiguration && waiting && <p>{settingsText.hint}</p>}
						{!secret && !isConfiguration && !invite && (
							<p className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs">
								<ToolChip tool={approval.tool} />
								<span>
									{approval.scopes.length > 0
										? target
										: t("crew.approval.askEveryTime", "Always asks first")}
								</span>
							</p>
						)}
						<RequesterLine approval={approval} />
						{flagged && waiting && <WebContentNotice />}
						{closure === "expired" && (
							<p className="mt-1 text-xs">
								{t(
									"crew.approval.expiredHint",
									"Nobody answered within 7 days, so {{name}} stopped.",
									{ name: botName },
								)}
							</p>
						)}
					</>
				}
			/>
			{invite && (
				<RequestCardBody dense>
					<InviteRequestDetails invite={invite} room={approval.room} />
				</RequestCardBody>
			)}
			{approval.configurationChanges && (
				<RequestCardBody className="max-h-[min(32rem,60vh)] overflow-y-auto overscroll-contain px-3.5 py-2.5">
					<BotConfigurationChanges
						changes={approval.configurationChanges}
						tool={approval.tool}
					/>
				</RequestCardBody>
			)}
			{waiting && secret && (
				<SecretRequestForm
					secret={secret}
					botName={botName}
					disabled={disabled}
					sealing={sealing}
					onSave={saveSecret}
					onDecline={() => answer("deny")}
				/>
			)}
			{waiting && !secret && (
				<RequestCardFooter
					className={
						isConfiguration
							? "px-3.5 py-2.5 [&>div:last-child]:max-w-full [&>div:last-child]:flex-wrap [&>div:last-child]:justify-end"
							: `px-3.5 py-2.5 ${NARROW_FOOTER}`
					}
					start={
						state.elsewhere ? (
							<p className="text-xs text-muted-foreground">
								{t(
									"crew.approval.answeredElsewhere",
									"Answered on another device",
								)}
							</p>
						) : (
							state.failed && (
								<p role="alert" className="text-xs text-destructive">
									{t(
										"crew.approval.failed",
										"Couldn't send your answer. Try again.",
									)}
								</p>
							)
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
							? settingsText.reject
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
						{invite ? <UserPlusIcon aria-hidden /> : <CheckIcon aria-hidden />}
						{isConfiguration
							? settingsText.approve
							: invite
								? t("crew.approval.invite.allow", "Invite")
								: t("crew.approval.once", "Allow once")}
					</Button>
					{rememberable && (
						<Button
							type="button"
							size="sm"
							className={`${requestButtonClass} @max-sm:basis-full`}
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
