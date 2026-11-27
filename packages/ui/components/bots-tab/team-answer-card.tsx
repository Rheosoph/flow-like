"use client";

import { useTranslation } from "@flow-like/locales";
import { BotIcon, MessagesSquareIcon } from "lucide-react";
import { Suspense, lazy } from "react";
import { useOrg } from "../../hooks/use-bot-teams";
import { useBotLabels } from "../../hooks/use-bots";
import { orgBot, teamById } from "../../lib/bot-teams/org-model";
import { BotAvatar } from "../bots/bot-avatar";
import { Button } from "../ui/button";
import type { ConversationTeamAnswer } from "./fold-entries";

const TextEditor = lazy(() =>
	import("../ui/text-editor").then((m) => ({ default: m.TextEditor })),
);

function LeadTile({
	appearance,
}: Readonly<{ appearance?: Parameters<typeof BotAvatar>[0]["appearance"] }>) {
	return (
		<span className="flex size-9 shrink-0 items-end justify-center overflow-hidden rounded-[10px] bg-muted">
			{appearance ? (
				<BotAvatar
					appearance={appearance}
					size={36}
					animated={false}
					decorative
				/>
			) : (
				<BotIcon
					className="size-5 self-center text-muted-foreground"
					aria-hidden
				/>
			)}
		</span>
	);
}

/** A team lead's answer to a `#Team` request, shown in the chat the request was written in. */
export function TeamAnswerCard({
	answer,
	onOpenThread,
}: Readonly<{ answer: ConversationTeamAnswer; onOpenThread: () => void }>) {
	const { t } = useTranslation("bots");
	const { org } = useOrg();
	const labels = useBotLabels();
	const lead = orgBot(org, answer.leadId);
	const team = teamById(org, answer.teamId);
	const leadName =
		(lead && labels({ system: lead.system ?? null, name: lead.name }).name) ||
		t("teams.requests.answer.leadFallback", "The lead");
	const teamName = team?.name ?? t("teams.requests.unknownTeam", "a team");
	return (
		<div className="flex w-full justify-center px-3">
			<article
				data-slot="team-answer"
				className="flex w-full max-w-prose flex-col gap-2 rounded-xl border bg-card p-3 text-card-foreground"
			>
				<header className="flex items-center gap-2.5">
					<LeadTile appearance={lead?.appearance} />
					<p className="min-w-0 flex-1 truncate text-sm font-medium">
						{t("teams.requests.answer.title", "{{lead}} · lead of {{team}}", {
							lead: leadName,
							team: teamName,
						})}
					</p>
				</header>
				{answer.text.trim() && (
					<div data-fl-chat-prose className="max-h-96 overflow-y-auto text-sm">
						<Suspense
							fallback={<p className="whitespace-pre-wrap">{answer.text}</p>}
						>
							<TextEditor
								presentation="chat"
								initialContent={answer.text}
								isMarkdown={true}
								editable={false}
							/>
						</Suspense>
					</div>
				)}
				<div className="flex justify-end">
					<Button
						type="button"
						variant="outline"
						size="sm"
						className="gap-1.5"
						onClick={onOpenThread}
					>
						<MessagesSquareIcon className="size-3.5" aria-hidden />
						{t("teams.requests.answer.open", "Open thread")}
					</Button>
				</div>
			</article>
		</div>
	);
}
