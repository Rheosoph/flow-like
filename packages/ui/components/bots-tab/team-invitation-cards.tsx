"use client";

import { useTranslation } from "@flow-like/locales";
import { UsersIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useTeamActions } from "../../hooks/use-bot-teams";
import { useBotLabels } from "../../hooks/use-bots";
import { apiErrorMessage } from "../../lib/api-error";
import { orgBot } from "../../lib/bot-teams/org-model";
import type {
	IOrgView,
	ITeamInvitation,
} from "../../state/backend-state/bot-team-state";
import type { IBotRoomLimits } from "../../state/backend-state/inbox-state";
import { IdentityAvatar } from "../identity/identity-avatar";
import { IdentityHoverCard } from "../identity/identity-hover-card";
import { Button } from "../ui/button";
import { NeedsYouCard } from "./needs-you-section";
import { RoomLimitsDialog } from "./room-limits-dialog";
import type { CrewDirectory } from "./use-crew-directory";

const DISMISSED_KEY = "flow-like:bots:dismissed-direct-asks";
const LEAD_TURNS_PER_PERSON_PER_DAY = 20;

export function teamInvitationKey(invitation: ITeamInvitation): string {
	return `${invitation.kind}:${invitation.team_id}:${invitation.bot_id ?? ""}:${invitation.user_id ?? ""}`;
}

function readDismissed(): string[] {
	try {
		const raw = localStorage.getItem(DISMISSED_KEY);
		const parsed: unknown = raw ? JSON.parse(raw) : [];
		return Array.isArray(parsed)
			? parsed.filter((item): item is string => typeof item === "string")
			: [];
	} catch {
		return [];
	}
}

/** The invitations to answer; "Not now" on a direct-asks request hides it on this device only. */
export function useTeamInvitations(org: IOrgView) {
	const [dismissed, setDismissed] = useState<readonly string[]>(readDismissed);
	const items = useMemo(
		() =>
			org.invitations.filter(
				(invitation) => !dismissed.includes(teamInvitationKey(invitation)),
			),
		[org.invitations, dismissed],
	);
	const dismiss = useCallback((invitation: ITeamInvitation) => {
		const key = teamInvitationKey(invitation);
		setDismissed((current) => {
			const next = [...current, key];
			try {
				localStorage.setItem(DISMISSED_KEY, JSON.stringify(next));
			} catch {
				// Hiding it for this session still works without storage.
			}
			return next;
		});
	}, []);
	return { items, dismiss };
}

function TeamInvitationCard({
	invitation,
	org,
	directory,
	onDismiss,
}: Readonly<{
	invitation: ITeamInvitation;
	org: IOrgView;
	directory: CrewDirectory;
	onDismiss: (invitation: ITeamInvitation) => void;
}>) {
	const { t } = useTranslation("bots");
	const labels = useBotLabels();
	const actions = useTeamActions();
	const {
		kind,
		team_id: teamId,
		team_name: team,
		org_name: orgName,
	} = invitation;
	const botId = invitation.bot_id ?? "";
	const bot = orgBot(org, botId);
	const botName = bot
		? labels({ system: bot.system ?? null, name: bot.name }).name
		: t("teams.chats.invite.aBot", "your bot");
	const inviterId = invitation.invited_by ?? "";
	const inviter = inviterId
		? directory(inviterId).name
		: t("teams.chats.invite.someone", "Someone");
	const pending =
		actions.acceptTeamInvite.isPending ||
		actions.declineTeamInvite.isPending ||
		actions.acceptBotInvite.isPending ||
		actions.declineBotInvite.isPending ||
		actions.setDirectAgreement.isPending ||
		actions.acceptLead.isPending ||
		actions.declineLead.isPending ||
		actions.acceptChat.isPending ||
		actions.declineChat.isPending;
	const [limitsOpen, setLimitsOpen] = useState(false);
	const requesterId = invitation.user_id ?? "";
	const requester = requesterId
		? directory(requesterId).name
		: t("teams.chats.invite.someone", "Someone");
	const askerId = kind === "chat_request" ? requesterId : inviterId;

	const run = (task: Promise<unknown>) =>
		task.catch((error: unknown) => {
			toast.error(
				apiErrorMessage(
					error,
					t(
						"teams.chats.invite.failed",
						"Couldn't answer the team invitation.",
					),
				),
			);
		});
	const accept = (limits?: IBotRoomLimits | null) => {
		if (kind === "lead")
			return run(actions.acceptLead.mutateAsync({ teamId, botId }));
		if (kind === "chat_request")
			return run(
				actions.acceptChat
					.mutateAsync({ botId, userId: requesterId, limits })
					.then(() => setLimitsOpen(false)),
			);
		if (kind === "person")
			return run(actions.acceptTeamInvite.mutateAsync(teamId));
		if (kind === "bot")
			return run(actions.acceptBotInvite.mutateAsync({ teamId, botId }));
		return run(
			actions.setDirectAgreement.mutateAsync({ teamId, botId, agreed: true }),
		);
	};
	const decline = () => {
		if (kind === "lead")
			return run(actions.declineLead.mutateAsync({ teamId, botId }));
		if (kind === "chat_request")
			return run(
				actions.declineChat.mutateAsync({ botId, userId: requesterId }),
			);
		if (kind === "person")
			return run(actions.declineTeamInvite.mutateAsync(teamId));
		if (kind === "bot")
			return run(actions.declineBotInvite.mutateAsync({ teamId, botId }));
		onDismiss(invitation);
	};

	let title: string;
	let meta: string;
	if (kind === "person") {
		title = t(
			"teams.chats.invite.personTitle",
			"{{name}} invited you to {{team}}",
			{
				name: inviter,
				team,
			},
		);
		meta = t(
			"teams.chats.invite.personMeta",
			"You will see every team in {{org}}",
			{ org: orgName },
		);
	} else if (kind === "lead") {
		title = t("teams.needs.leadTitle", "{{team}} asks {{bot}} to lead it", {
			team,
			bot: botName,
		});
		meta = t(
			"teams.needs.leadMeta",
			"Everyone in {{org}} can then talk to it, and its turns are on you (up to {{limit}} per person a day).",
			{ org: orgName, limit: LEAD_TURNS_PER_PERSON_PER_DAY },
		);
	} else if (kind === "chat_request") {
		title = t("teams.needs.chatTitle", "{{name}} wants to talk with {{bot}}", {
			name: requester,
			bot: botName,
		});
		meta =
			invitation.note?.trim() || t("teams.needs.chatNoNote", "No note added");
	} else if (kind === "bot") {
		title = t(
			"teams.chats.invite.botTitle",
			"{{name}} wants to add {{bot}} to {{team}}",
			{ name: inviter, bot: botName, team },
		);
		meta = t(
			"teams.chats.invite.botMeta",
			"{{bot}} reads the team's brief once it joins",
			{ bot: botName },
		);
	} else {
		title = t(
			"teams.chats.invite.directTitle",
			"{{team}} lets teammates ask {{bot}} directly",
			{ team, bot: botName },
		);
		meta = t(
			"teams.chats.invite.directMeta",
			"They skip the first-time Allow card. You can stop it later.",
		);
	}

	return (
		<>
			<NeedsYouCard
				avatar={
					askerId ? (
						<IdentityHoverCard subject={askerId}>
							<IdentityAvatar subject={askerId} size={36} />
						</IdentityHoverCard>
					) : (
						<span className="flex size-9 shrink-0 items-center justify-center rounded-[10px] bg-muted text-muted-foreground">
							<UsersIcon className="size-4" aria-hidden />
						</span>
					)
				}
				name={kind === "chat_request" ? requester : inviter}
				where={t("teams.chats.invite.where", "{{team}} · {{org}}", {
					team,
					org: orgName,
				})}
				title={title}
				meta={meta}
				actions={
					<>
						<Button
							type="button"
							size="sm"
							className="rounded-[9px]"
							disabled={pending}
							onClick={() =>
								kind === "chat_request" ? setLimitsOpen(true) : void accept()
							}
						>
							{kind === "direct_asks"
								? t("teams.chats.invite.allow", "Allow")
								: kind === "chat_request"
									? t("teams.needs.approve", "Approve")
									: t("teams.chats.invite.accept", "Accept")}
						</Button>
						<Button
							type="button"
							variant="outline"
							size="sm"
							className="rounded-[9px]"
							disabled={pending}
							onClick={() => void decline()}
						>
							{kind === "direct_asks"
								? t("teams.chats.invite.notNow", "Not now")
								: t("teams.chats.invite.decline", "Decline")}
						</Button>
					</>
				}
			/>
			{kind === "chat_request" && (
				<RoomLimitsDialog
					open={limitsOpen}
					onOpenChange={setLimitsOpen}
					botId={botId}
					botName={botName}
					roomName={requester}
					group={false}
					confirmLabel={t("teams.needs.approveConfirm", "Approve and allow")}
					pending={actions.acceptChat.isPending}
					error={actions.acceptChat.error}
					onConfirm={(limits) => void accept(limits)}
				/>
			)}
		</>
	);
}

export function TeamInvitationCards({
	invitations,
	org,
	directory,
	onDismiss,
}: Readonly<{
	invitations: readonly ITeamInvitation[];
	org: IOrgView;
	directory: CrewDirectory;
	onDismiss: (invitation: ITeamInvitation) => void;
}>) {
	return (
		<>
			{invitations.map((invitation) => (
				<TeamInvitationCard
					key={teamInvitationKey(invitation)}
					invitation={invitation}
					org={org}
					directory={directory}
					onDismiss={onDismiss}
				/>
			))}
		</>
	);
}
