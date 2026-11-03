"use client";

import { useTranslation } from "@flow-like/locales";
import { type ReactNode, useState } from "react";
import { toast } from "sonner";
import { useAnswerInvitation } from "../../hooks/use-inbox";
import { apiErrorMessage } from "../../lib/api-error";
import type {
	IBotRoomLimits,
	IConversation,
	IInvitation,
} from "../../state/backend-state/inbox-state";
import { BotAvatar } from "../bots/bot-avatar";
import type { BotAppearance } from "../bots/types";
import { IdentityAvatar } from "../identity/identity-avatar";
import { Button } from "../ui/button";
import { isApprovalCard } from "./approval-entry";
import { CrewSection, StatusDot } from "./crew-section";
import { RoomLimitsDialog } from "./room-limits-dialog";
import type { ConversationText } from "./use-conversation-text";
import type { CrewDirectory } from "./use-crew-directory";

export type NeedsYouItem =
	| { type: "conversation"; conversation: IConversation }
	| { type: "invitation"; invitation: IInvitation }
	| {
			type: "assistant";
			name: string;
			appearance?: Partial<BotAppearance>;
	  };

function NeedsYouCard({
	avatar,
	name,
	where,
	title,
	meta,
	onOpen,
	actions,
}: Readonly<{
	avatar: ReactNode;
	name: string;
	where: string;
	title: string;
	meta: string;
	onOpen?: () => void;
	actions?: ReactNode;
}>) {
	const { t } = useTranslation("bots");
	return (
		<article className="flex flex-col gap-2.5 rounded-2xl border border-primary/40 bg-card p-3.5">
			<div className="flex items-center gap-2.5">
				{avatar}
				<div className="min-w-0 flex-1">
					<div className="truncate text-[13px] font-semibold">{name}</div>
					<div className="truncate text-xs text-muted-foreground">{where}</div>
				</div>
				<StatusDot pulse />
			</div>
			<div>
				<div className="line-clamp-2 text-sm font-semibold leading-snug">
					{title}
				</div>
				<div className="mt-0.5 text-[13px] text-muted-foreground">{meta}</div>
			</div>
			<div className="mt-auto flex flex-wrap gap-2">
				{actions ?? (
					<Button
						type="button"
						variant="outline"
						size="sm"
						className="rounded-[9px]"
						onClick={onOpen}
					>
						{t("crew.openChat", "Open chat")}
					</Button>
				)}
			</div>
		</article>
	);
}

function InvitationNeed({
	invitation,
	directory,
}: Readonly<{ invitation: IInvitation; directory: CrewDirectory }>) {
	const { t } = useTranslation("bots");
	const answer = useAnswerInvitation();
	const [limitsOpen, setLimitsOpen] = useState(false);
	const forBot = invitation.kind === "bot";
	const inviter = directory(invitation.invited_by ?? "").name;
	const room =
		invitation.title ?? t("crew.invitations.groupChat", "a group chat");
	const botName = invitation.bot_name ?? t("crew.invitations.aBot", "your bot");

	const respond = (accept: boolean, limits?: IBotRoomLimits | null) =>
		answer
			.mutateAsync({ invitation, accept, limits })
			.then(() => setLimitsOpen(false))
			.catch((error: unknown) => {
				toast.error(
					apiErrorMessage(
						error,
						t("crew.invitations.failed", "Couldn't answer the invitation."),
					),
				);
			});

	return (
		<>
			<NeedsYouCard
				avatar={
					<IdentityAvatar subject={invitation.invited_by ?? ""} size={36} />
				}
				name={inviter}
				where={room}
				title={
					forBot
						? t(
								"crew.invitations.botTitle",
								"{{name}} wants to add {{bot}} to {{room}}",
								{ name: inviter, bot: botName, room },
							)
						: t("crew.invitations.title", "{{name}} invited you to {{room}}", {
								name: inviter,
								room,
							})
				}
				meta={
					forBot
						? t(
								"crew.invitations.botMeta",
								"You choose what {{bot}} may do there",
								{ bot: botName },
							)
						: t("crew.invitations.meta", "Join to read and write in it")
				}
				actions={
					<>
						<Button
							type="button"
							size="sm"
							className="rounded-[9px]"
							disabled={answer.isPending}
							onClick={() =>
								forBot ? setLimitsOpen(true) : void respond(true)
							}
						>
							{forBot
								? t("crew.invitations.review", "Review and accept")
								: t("crew.invitations.accept", "Accept")}
						</Button>
						<Button
							type="button"
							variant="outline"
							size="sm"
							className="rounded-[9px]"
							disabled={answer.isPending}
							onClick={() => void respond(false)}
						>
							{t("crew.invitations.decline", "Decline")}
						</Button>
					</>
				}
			/>
			{forBot && (
				<RoomLimitsDialog
					open={limitsOpen}
					onOpenChange={setLimitsOpen}
					botId={invitation.principal_id}
					botName={botName}
					roomName={room}
					confirmLabel={t("crew.invitations.acceptBot", "Accept and add")}
					pending={answer.isPending}
					error={answer.error}
					onConfirm={(limits) => void respond(true, limits)}
				/>
			)}
		</>
	);
}

export function NeedsYouSection({
	items,
	directory,
	text,
	onOpenConversation,
	onOpenAssistant,
}: Readonly<{
	items: readonly NeedsYouItem[];
	directory: CrewDirectory;
	text: ConversationText;
	onOpenConversation: (conversationId: string) => void;
	onOpenAssistant: () => void;
}>) {
	const { t } = useTranslation("bots");
	if (items.length === 0) return null;
	return (
		<CrewSection
			title={t("crew.needs.title", "Needs you")}
			aside={
				<span className="inline-flex h-5 min-w-5 items-center justify-center self-center rounded-full bg-primary/15 px-1.5 text-xs font-semibold text-primary">
					{items.length}
				</span>
			}
		>
			<div className="grid grid-cols-[repeat(auto-fill,minmax(min(250px,100%),1fr))] gap-3">
				{items.map((item) => {
					if (item.type === "invitation") {
						return (
							<InvitationNeed
								key={item.invitation.id}
								invitation={item.invitation}
								directory={directory}
							/>
						);
					}
					if (item.type === "assistant") {
						return (
							<NeedsYouCard
								key="assistant"
								avatar={
									<span className="flex size-9 shrink-0 items-end justify-center overflow-hidden rounded-[10px] bg-muted">
										<BotAvatar
											appearance={item.appearance}
											size={36}
											animated={false}
											decorative
										/>
									</span>
								}
								name={item.name}
								where={t("crew.needs.assistantWhere", "Your chat")}
								title={t(
									"crew.needs.assistantTitle",
									"Waiting for your answer",
								)}
								meta={t(
									"crew.needs.assistantMeta",
									"It paused until you decide",
								)}
								onOpen={onOpenAssistant}
							/>
						);
					}
					const { conversation } = item;
					const sender = conversation.last_entry?.sender_id ?? "";
					const when = text.activity(
						conversation.last_entry?.created_at ??
							conversation.last_activity_at,
					);
					return (
						<NeedsYouCard
							key={conversation.id}
							avatar={<IdentityAvatar subject={sender} size={36} />}
							name={directory(sender).name}
							where={
								conversation.kind === "group"
									? text.title(conversation)
									: t("crew.directMessage", "Direct message")
							}
							title={text.entryText(conversation.last_entry)}
							meta={
								isApprovalCard(conversation.last_entry)
									? t("crew.needs.approvalMeta", "Waiting for you · {{time}}", {
											time: when,
										})
									: t("crew.needs.meta", "{{count}} new · {{time}}", {
											count: conversation.unread_count,
											time: when,
										})
							}
							onOpen={() => onOpenConversation(conversation.id)}
						/>
					);
				})}
			</div>
		</CrewSection>
	);
}
