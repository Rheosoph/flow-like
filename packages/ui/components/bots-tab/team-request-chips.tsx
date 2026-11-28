"use client";

import { useTranslation } from "@flow-like/locales";
import { useOrg } from "../../hooks/use-bot-teams";
import { teamById, teamTag } from "../../lib/bot-teams/org-model";
import { cn } from "../../lib/utils";
import { useOpenConversation } from "./conversation-note";

const CHIP =
	"inline-flex max-w-full items-center rounded-full bg-muted px-2 py-0.5 text-[11px] text-muted-foreground";

/**
 * "→ #Security" under a message that went to teams. The sender's chip opens their thread with
 * that team when they have one; for everyone else it is a label.
 */
export function TeamRequestChips({
	teamIds,
	mine,
}: Readonly<{ teamIds: readonly string[]; mine: boolean }>) {
	const { t } = useTranslation("bots");
	const { org } = useOrg();
	const openConversation = useOpenConversation();
	const threads = org.threads ?? [];
	return (
		<div data-slot="team-request-chips" className="flex flex-wrap gap-1.5 px-4">
			{teamIds.map((id) => {
				const team = teamById(org, id);
				const label = t("teams.requests.chip", "→ {{team}}", {
					team: team
						? teamTag(team.name)
						: t("teams.requests.unknownTeam", "a team"),
				});
				const thread = mine
					? threads.find((item) => item.team_id === id)
					: undefined;
				if (!thread) {
					return (
						<span key={id} className={CHIP}>
							<span className="truncate">{label}</span>
						</span>
					);
				}
				return (
					<button
						key={id}
						type="button"
						title={t(
							"teams.requests.chipOpen",
							"Open your thread with {{team}}",
							{
								team: team?.name ?? t("teams.requests.unknownTeam", "a team"),
							},
						)}
						onClick={() => openConversation(thread.conversation_id)}
						className={cn(
							CHIP,
							"outline-none transition-colors hover:bg-primary/10 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
						)}
					>
						<span className="truncate">{label}</span>
					</button>
				);
			})}
		</div>
	);
}
