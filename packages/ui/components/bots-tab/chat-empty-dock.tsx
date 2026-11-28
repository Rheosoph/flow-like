"use client";

import { useTranslation } from "@flow-like/locales";
import { MessagesSquareIcon, UsersIcon } from "lucide-react";
import { teamTag } from "../../lib/bot-teams/org-model";
import { Button } from "../ui/button";
import type { TeamRoom } from "./team-chat-list-model";

/** The dock's place while no chat is open: team rooms are the quickest way to start one. */
export function ChatEmptyDock({
	rooms,
	onOpenConversation,
}: Readonly<{
	rooms: readonly TeamRoom[];
	onOpenConversation: (conversationId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<div className="flex flex-1 flex-col items-center justify-center gap-4 px-6 text-center">
			<span className="flex size-12 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
				<MessagesSquareIcon className="size-6" aria-hidden />
			</span>
			<div className="flex flex-col gap-1">
				<h2 className="text-base font-semibold">
					{t("teams.chats.empty.title", "Pick a chat, or message a team")}
				</h2>
				<p className="max-w-sm text-sm text-muted-foreground">
					{rooms.length > 0
						? t(
								"teams.chats.empty.withTeams",
								"A message in a team room reaches the team's lead first, who answers or hands it on.",
							)
						: t(
								"teams.chats.empty.body",
								"Open a conversation from the list to read and write here.",
							)}
				</p>
			</div>
			{rooms.length > 0 && (
				<div className="flex max-w-md flex-wrap justify-center gap-2">
					{rooms.map(({ team, conversation }) => (
						<Button
							key={team.id}
							type="button"
							variant="outline"
							size="sm"
							className="rounded-full"
							onClick={() => onOpenConversation(conversation.id)}
						>
							<UsersIcon aria-hidden />
							{team.name}
							<span className="text-xs font-normal text-muted-foreground">
								{teamTag(team.name)}
							</span>
						</Button>
					))}
				</div>
			)}
		</div>
	);
}
