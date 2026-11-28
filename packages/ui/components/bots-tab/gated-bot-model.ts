import { isGatedInRoom, reachOf } from "../../lib/bot-teams/org-model";
import type { IOrgView } from "../../state/backend-state/bot-team-state";

/**
 * Whether a bot in a team conversation (team room, the reader's thread with a team, or a bot chat)
 * takes no turn there: the room's gate holds it back, or the reader's classification doesn't reach it.
 */
export function silentInConversation(
	org: IOrgView,
	conversationId: string,
	botId: string,
	me?: string | null,
): boolean {
	const room = org.teams.find(
		(team) => team.conversation_id === conversationId,
	);
	if (room) return isGatedInRoom(room, botId);
	const inThread = (org.threads ?? []).some(
		(thread) => thread.conversation_id === conversationId,
	);
	const inChat = (org.chats ?? []).some(
		(chat) => chat.conversation_id === conversationId && chat.bot_id === botId,
	);
	return (inThread || inChat) && reachOf(org, me, botId).reach === "classified";
}
