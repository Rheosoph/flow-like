import { orgBot, teamTag } from "../../lib/bot-teams/org-model";
import type { IOrgView } from "../../state/backend-state/bot-team-state";

export interface TeamTagOption {
	id: string;
	name: string;
	/** The tag without its `#`. */
	tag: string;
	/** The team room's picture, the team's icon. */
	imageUrl?: string;
	leadName?: string;
	/** The lead's classification when the reader can't talk to it because of it; the option is listed but can't be picked. */
	lockedLevel?: string;
}

export interface TeamTagQuery {
	start: number;
	end: number;
	/** What follows the `#` up to the end of the token. */
	query: string;
}

const MAX_SUGGESTIONS = 6;

/**
 * The leading `#tag` token the caret sits in or right behind, if any. Only tokens at the very
 * start of the text count, as on the server: after the first word that isn't a `#tag` none do.
 */
export function leadingTagQuery(
	text: string,
	caret: number,
): TeamTagQuery | null {
	let index = 0;
	while (index < text.length) {
		while (index < text.length && /\s/.test(text[index])) index++;
		if (text[index] !== "#") return null;
		let end = index;
		while (end < text.length && !/\s/.test(text[end])) end++;
		if (caret >= index && caret <= end) {
			return { start: index, end, query: text.slice(index + 1, end) };
		}
		index = end;
	}
	return null;
}

export function matchTeamTags(
	options: readonly TeamTagOption[],
	query: string,
): TeamTagOption[] {
	const needle = query.toLowerCase();
	return options
		.filter((option) => option.tag.toLowerCase().startsWith(needle))
		.sort(
			(a, b) =>
				Number(Boolean(a.lockedLevel)) - Number(Boolean(b.lockedLevel)) ||
				a.tag.localeCompare(b.tag),
		)
		.slice(0, MAX_SUGGESTIONS);
}

/** Replaces the token with `#Tag ` and returns the new caret; a space already after it is reused. */
export function insertTeamTag(
	text: string,
	range: TeamTagQuery,
	tag: string,
): { text: string; caret: number } {
	const token = `#${tag} `;
	const rest = text.slice(range.end);
	const tail = /^[ \t]/.test(rest) ? rest.slice(1) : rest;
	return {
		text: text.slice(0, range.start) + token + tail,
		caret: range.start + token.length,
	};
}

/**
 * The teams a message can address: those with a lead, since nobody else answers a request. A lead
 * the reader's classification doesn't reach stays listed, marked with its level.
 */
export function teamTagOptions(
	org: IOrgView,
	leadName: (botId: string) => string | undefined,
): TeamTagOption[] {
	return org.teams.flatMap((team) => {
		const tag = teamTag(team.name).slice(1);
		if (!tag || !team.lead_bot_id) return [];
		const lead = leadName(team.lead_bot_id);
		const classified = org.reach?.some(
			(entry) =>
				entry.bot_id === team.lead_bot_id && entry.reach === "classified",
		);
		const lockedLevel = classified
			? (orgBot(org, team.lead_bot_id)?.classification ?? "internal")
			: undefined;
		return [
			{
				id: team.id,
				name: team.name,
				tag,
				...(team.image_url ? { imageUrl: team.image_url } : {}),
				...(lead ? { leadName: lead } : {}),
				...(lockedLevel ? { lockedLevel } : {}),
			},
		];
	});
}

const MAX_REQUEST_TEAMS = 3;

/**
 * The `#tags` a message starts with, as the server reads them: leading tokens that begin with `#`,
 * without trailing punctuation, at most three. A tag later in the text addresses no team.
 */
export function leadingTags(text: string): string[] {
	const tags: string[] = [];
	for (const token of text.trim().split(/\s+/)) {
		if (!token.startsWith("#")) break;
		const tag = token.slice(1).replace(/[,:;.!?]+$/, "");
		if (!tag || tag.startsWith("#")) break;
		if (!tags.some((known) => known.toLowerCase() === tag.toLowerCase()))
			tags.push(tag);
		if (tags.length === MAX_REQUEST_TEAMS) break;
	}
	return tags;
}

/** Whether `text` starts with the tag of one of `options`: it goes to that team, not this chat's bot. */
export function addressesTeam(
	text: string,
	options: readonly TeamTagOption[],
): boolean {
	return leadingTags(text).some((tag) =>
		options.some(
			(option) =>
				!option.lockedLevel && option.tag.toLowerCase() === tag.toLowerCase(),
		),
	);
}
