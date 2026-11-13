"use client";

import { useTranslation } from "@flow-like/locales";
import type { IInboxApprovalInvite } from "../../state/backend-state/inbox-state";
import { RequestDetails } from "../interfaces/chat-default/request-card";

/** Why a bot wants to bring someone into the room, as the owner reads it before deciding. */
export function InviteRequestDetails({
	invite,
	room,
}: Readonly<{ invite: IInboxApprovalInvite; room?: string }>) {
	const { t } = useTranslation("bots");
	return (
		<div data-slot="approval-invite" className="space-y-1.5">
			<p className="text-xs font-medium text-foreground/80">
				{t("crew.approval.invite.why", "Why")}
			</p>
			<RequestDetails text={invite.reason} />
			<p className="text-xs text-muted-foreground">
				{room
					? t(
							"crew.approval.invite.nextInRoom",
							"They join {{room}} once they accept. Asked every time.",
							{ room },
						)
					: t(
							"crew.approval.invite.next",
							"They join once they accept. Asked every time.",
						)}
			</p>
		</div>
	);
}
