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
import { IdentityAvatar } from "../identity/identity-avatar";
import { IdentityHoverCard } from "../identity/identity-hover-card";
import { Button } from "../ui/button";
import { NeedsYouCard } from "./needs-you-section";
import type { CrewDirectory } from "./use-crew-directory";

const DISMISSED_KEY = "flow-like:bots:dismissed-direct-asks";

export function teamInvitationKey(invitation: ITeamInvitation): string {
	return `${invitation.kind}:${invitation.team_id}:${invitation.bot_id ?? ""}`;
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
		actions.setDirectAgreement.isPending;

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
	const accept = () => {
		if (kind === "person")
			return run(actions.acceptTeamInvite.mutateAsync(teamId));
		if (kind === "bot")
			return run(actions.acceptBotInvite.mutateAsync({ teamId, botId }));
		return run(
			actions.setDirectAgreement.mutateAsync({ teamId, botId, agreed: true }),
		);
	};
	const decline = () => {
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
		<NeedsYouCard
			avatar={
				inviterId ? (
					<IdentityHoverCard subject={inviterId}>
						<IdentityAvatar subject={inviterId} size={36} />
					</IdentityHoverCard>
				) : (
					<span className="flex size-9 shrink-0 items-center justify-center rounded-[10px] bg-muted text-muted-foreground">
						<UsersIcon className="size-4" aria-hidden />
					</span>
				)
			}
			name={inviter}
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
						onClick={() => void accept()}
					>
						{kind === "direct_asks"
							? t("teams.chats.invite.allow", "Allow")
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
