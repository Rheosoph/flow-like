import type { IConversation } from "../../state/backend-state/inbox-state";
import { isSelfPrincipal } from "./crew-model";
import type { CrewDirectory } from "./use-crew-directory";

export interface Mentionable {
	id: string;
	name: string;
	kind: "user" | "bot";
}

export interface MentionQuery {
	start: number;
	query: string;
}

const TRIGGER = /(^|\s)@([^\s@]*)$/;
const MAX_SUGGESTIONS = 6;

/** The `@word` being typed right before the caret, if any. */
export function mentionQuery(text: string, caret: number): MentionQuery | null {
	const match = TRIGGER.exec(text.slice(0, caret));
	if (!match) return null;
	return {
		start: caret - match[2].length - 1,
		query: match[2],
	};
}

/** Replaces the `@word` at `start..caret` with `@Name ` and returns the new caret. */
export function insertMention(
	text: string,
	range: MentionQuery,
	caret: number,
	name: string,
): { text: string; caret: number } {
	const token = `@${name} `;
	return {
		text: text.slice(0, range.start) + token + text.slice(caret),
		caret: range.start + token.length,
	};
}

export function filterMentionables(
	options: readonly Mentionable[],
	query: string,
): Mentionable[] {
	const needle = query.trim().toLowerCase();
	return options
		.filter((option) => option.name.toLowerCase().includes(needle))
		.sort(
			(a, b) =>
				Number(b.name.toLowerCase().startsWith(needle)) -
				Number(a.name.toLowerCase().startsWith(needle)),
		)
		.slice(0, MAX_SUGGESTIONS);
}

/** The picked principals whose `@Name` is still in the text, in the order picked. */
export function mentionsIn(
	text: string,
	picked: ReadonlyMap<string, string>,
): string[] {
	return [...picked]
		.filter(([, name]) => text.includes(`@${name}`))
		.map(([id]) => id);
}

/** Who a room's composer suggests: its active members other than you, people and bots. */
export function mentionablesOf(
	conversation: IConversation,
	directory: CrewDirectory,
	me?: string | null,
): Mentionable[] {
	if (conversation.kind !== "group") return [];
	return conversation.members
		.filter((member) => !isSelfPrincipal(member.principal_id, me))
		.map((member) => ({
			id: member.principal_id,
			name: directory(member.principal_id).name,
			kind: member.kind,
		}));
}
