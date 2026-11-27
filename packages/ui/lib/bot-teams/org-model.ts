import type {
	IOrgBot,
	IOrgView,
	ITeam,
	ITeamBot,
} from "../../state/backend-state/bot-team-state";

/** How one bot may reach another, the way the hub decides it. */
export type AskStatus =
	| "self"
	/** Same team, direct asks on and both owners agreed: no Allow card. */
	| "direct"
	/** Its owner allows the first question: a teammate, an own bot, or another team's lead. */
	| "ok"
	/** Another team's member: ask its lead, who hands it on. */
	| "via"
	/** Classified above the asking bot. */
	| "blocked"
	/** Invited and not yet accepted by its owner. */
	| "later"
	/** Outside the asking bot's reach. */
	| "none";

export interface OrgTeamNode {
	team: ITeam;
	depth: number;
	children: OrgTeamNode[];
}

const CLASSIFICATION_LEVEL: Record<string, number> = {
	public: 0,
	internal: 1,
	confidential: 2,
	restricted: 3,
};

/** A bot's classification level; unclassified counts as internal, as on the hub. */
export function classificationLevel(
	bot: Pick<IOrgBot, "classification">,
): number {
	return CLASSIFICATION_LEVEL[bot.classification ?? "internal"] ?? 1;
}

export function teamById(org: IOrgView, teamId: string): ITeam | undefined {
	return org.teams.find((team) => team.id === teamId);
}

export function orgBot(org: IOrgView, botId: string): IOrgBot | undefined {
	return org.bots.find((bot) => bot.id === botId);
}

export function childTeams(org: IOrgView, teamId: string | null): ITeam[] {
	return org.teams
		.filter((team) => (team.parent_id ?? null) === teamId)
		.sort((a, b) => a.name.localeCompare(b.name));
}

/** Top-level teams first, then each team's path from its top-level team down to it. */
export function teamPath(org: IOrgView, teamId: string): ITeam[] {
	const path: ITeam[] = [];
	const seen = new Set<string>();
	let team = teamById(org, teamId);
	while (team && !seen.has(team.id)) {
		seen.add(team.id);
		path.unshift(team);
		team = team.parent_id ? teamById(org, team.parent_id) : undefined;
	}
	return path;
}

export function rootTeamId(org: IOrgView, teamId: string): string | undefined {
	return teamPath(org, teamId)[0]?.id;
}

/** Every org as a tree, top-level teams in name order. */
export function orgForest(org: IOrgView): OrgTeamNode[] {
	const build = (
		team: ITeam,
		depth: number,
		seen: Set<string>,
	): OrgTeamNode => {
		seen.add(team.id);
		return {
			team,
			depth,
			children: childTeams(org, team.id)
				.filter((child) => !seen.has(child.id))
				.map((child) => build(child, depth + 1, seen)),
		};
	};
	const roots = org.teams
		.filter((team) => !team.parent_id || !teamById(org, team.parent_id))
		.sort((a, b) => a.name.localeCompare(b.name));
	return roots.map((root) => build(root, 0, new Set()));
}

/** Every team depth first, each with its depth: the order a list or an outline shows them in. */
export function flattenForest(nodes: OrgTeamNode[]): OrgTeamNode[] {
	return nodes.flatMap((node) => [node, ...flattenForest(node.children)]);
}

export function activeBots(team: ITeam): ITeamBot[] {
	return team.bots.filter((bot) => bot.status === "active");
}

/** The team a bot is active in; a bot is active in at most one. */
export function activeTeamOf(org: IOrgView, botId: string): ITeam | undefined {
	return org.teams.find((team) =>
		team.bots.some((bot) => bot.bot_id === botId && bot.status === "active"),
	);
}

/** Teams that invited the bot and wait for its owner. */
export function pendingTeamsOf(org: IOrgView, botId: string): ITeam[] {
	return org.teams.filter((team) =>
		team.bots.some((bot) => bot.bot_id === botId && bot.status === "pending"),
	);
}

export function isTeamAdmin(
	team: ITeam,
	userId: string | null | undefined,
): boolean {
	return Boolean(
		userId &&
			team.people.some(
				(person) =>
					person.user_id === userId &&
					person.role === "admin" &&
					person.status === "active",
			),
	);
}

export function isTeamMember(
	team: ITeam,
	userId: string | null | undefined,
): boolean {
	return Boolean(
		userId &&
			team.people.some(
				(person) => person.user_id === userId && person.status === "active",
			),
	);
}

/** The bot, its team's lead, and every lead above, as far as the org goes. */
export function reportingLine(org: IOrgView, botId: string): string[] {
	const line = [botId];
	let current = botId;
	let team = activeTeamOf(org, botId);
	const seen = new Set<string>();
	while (team && !seen.has(team.id)) {
		seen.add(team.id);
		if (team.lead_bot_id && team.lead_bot_id !== current) {
			line.push(team.lead_bot_id);
			current = team.lead_bot_id;
		}
		team = team.parent_id ? teamById(org, team.parent_id) : undefined;
	}
	return line;
}

function membership(team: ITeam, botId: string): ITeamBot | undefined {
	return team.bots.find((bot) => bot.bot_id === botId);
}

/**
 * How `fromId` may ask `toId`, by the hub's rules: classification first, then the team. Teammates
 * are asked directly when the team allows it and both owners agreed, otherwise with the asking
 * bot's owner's OK; another team's lead with that OK; another team's member through its lead.
 */
export function askStatus(
	org: IOrgView,
	fromId: string,
	toId: string,
): AskStatus {
	if (fromId === toId) return "self";
	const from = orgBot(org, fromId);
	const to = orgBot(org, toId);
	if (!from || !to) return "none";
	const toTeam = activeTeamOf(org, toId);
	if (!toTeam && pendingTeamsOf(org, toId).length > 0) return "later";
	if (classificationLevel(from) < classificationLevel(to)) return "blocked";
	const fromTeam = activeTeamOf(org, fromId);
	if (fromTeam && toTeam && fromTeam.id === toTeam.id) {
		const agreed =
			membership(fromTeam, fromId)?.direct_agreed === true &&
			membership(fromTeam, toId)?.direct_agreed === true;
		return fromTeam.direct_asks && agreed ? "direct" : "ok";
	}
	if (from.owner_id === to.owner_id) return "ok";
	if (
		fromTeam &&
		toTeam &&
		rootTeamId(org, fromTeam.id) === rootTeamId(org, toTeam.id)
	) {
		if (toTeam.lead_bot_id === toId) return "ok";
		const lead = toTeam.lead_bot_id;
		if (lead && ["ok", "direct"].includes(askStatus(org, fromId, lead)))
			return "via";
	}
	return "none";
}

/** The team room's tag: its name without spaces after `#`. */
export function teamTag(name: string): string {
	return `#${name.split(/\s+/).join("")}`;
}

/** Whether the conversation is a team's room or the reader's own thread with a team. */
export function isTeamConversation(
	org: IOrgView,
	conversationId: string,
): boolean {
	return (
		org.teams.some((team) => team.conversation_id === conversationId) ||
		(org.threads ?? []).some(
			(thread) => thread.conversation_id === conversationId,
		)
	);
}
