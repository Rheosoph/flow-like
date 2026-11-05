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

function escapeRegExp(value: string): string {
	return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * The principals whose `@Name` stands in the text as a whole word, picked from the menu or typed
 * out, so "@Scout" names Scout and "@Scouting" doesn't.
 */
export function mentionedIn(
	text: string,
	options: readonly Mentionable[],
): string[] {
	return options
		.filter(
			(option) =>
				option.name &&
				new RegExp(
					`(^|\\s)@${escapeRegExp(option.name)}(?=$|[\\s.,!?;:)])`,
					"u",
				).test(text),
		)
		.map((option) => option.id);
}

/**
 * The bots whose name is spoken as a whole word, in any case ("hey momo" names Momo, "momos"
 * doesn't). People are never matched: a spoken name in a call is just conversation.
 */
export function spokenMentions(
	text: string,
	options: readonly Mentionable[],
): string[] {
	return options
		.filter(
			(option) =>
				option.kind === "bot" &&
				option.name.trim() &&
				new RegExp(
					`(?<![\\p{L}\\p{N}_])${escapeRegExp(option.name.trim())}(?![\\p{L}\\p{N}_])`,
					"iu",
				).test(text),
		)
		.map((option) => option.id);
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

export interface MentionTarget {
	id: string;
	name: string;
}

/**
 * Turns `@Name` into a link the chat renderer shows as a highlighted name; a mentioned principal
 * the text doesn't spell out leads the first line instead.
 */
export function inlineMentions(
	text: string,
	targets: readonly MentionTarget[],
): string {
	let body = text;
	const missing: string[] = [];
	for (const { id, name } of targets) {
		if (!name) continue;
		const link = `[@${name}](mention://${encodeURIComponent(id)})`;
		const pattern = new RegExp(
			`(^|\\s)@${escapeRegExp(name)}(?=$|[\\s.,!?;:)])`,
			"gu",
		);
		if (pattern.test(body)) {
			body = body.replace(pattern, (_, lead: string) => `${lead}${link}`);
		} else {
			missing.push(link);
		}
	}
	return missing.length > 0 ? `${missing.join(" ")} ${body}` : body;
}

/** Direct calls address their sole bot; a group call requires a spoken bot name. */
export function callMentions(
	text: string,
	options: readonly Mentionable[],
	kind: IConversation["kind"],
): string[] {
	const bots = options.filter((option) => option.kind === "bot");
	if (kind === "direct" && bots.length === 1 && text.trim())
		return [bots[0].id];
	return spokenMentions(text, options);
}
