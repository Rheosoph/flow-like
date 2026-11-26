import {
	type AskStatus,
	activeBots,
	activeTeamOf,
	askStatus,
	flattenForest,
	isTeamAdmin,
	isTeamMember,
	orgBot,
	orgForest,
	pendingTeamsOf,
	reportingLine,
	rootTeamId,
	teamById,
	teamPath,
} from "../../../lib/bot-teams/org-model";
import type { IBot } from "../../../state/backend-state/bot-state";
import type {
	IOrgBot,
	IOrgView,
	ITeam,
} from "../../../state/backend-state/bot-team-state";

export const ASK_GROUP_ORDER = [
	"direct",
	"ok",
	"via",
	"blocked",
	"later",
] as const;
export type AskGroupKey = (typeof ASK_GROUP_ORDER)[number];

export type AskGroups = Record<AskGroupKey, IOrgBot[]> & { none: IOrgBot[] };

export interface Highlight {
	botIds: ReadonlySet<string>;
	teamIds: ReadonlySet<string>;
}

export const NO_HIGHLIGHT: Highlight = {
	botIds: new Set(),
	teamIds: new Set(),
};

export function highlightFor(org: IOrgView, botId: string | null): Highlight {
	if (!botId) return NO_HIGHLIGHT;
	const botIds = new Set(reportingLine(org, botId));
	const teamIds = new Set<string>();
	for (const id of botIds) {
		const team = activeTeamOf(org, id);
		if (team) for (const entry of teamPath(org, team.id)) teamIds.add(entry.id);
	}
	return { botIds, teamIds };
}

export function groupAsks(org: IOrgView, fromId: string): AskGroups {
	const groups: AskGroups = {
		direct: [],
		ok: [],
		via: [],
		blocked: [],
		later: [],
		none: [],
	};
	for (const bot of org.bots) {
		const status = askStatus(org, fromId, bot.id);
		if (status === "self") continue;
		groups[status].push(bot);
	}
	return groups;
}

export function askBadgeStatus(
	org: IOrgView,
	selectedId: string | null,
	botId: string,
): Exclude<AskStatus, "self" | "none"> | null {
	if (!selectedId) return null;
	const status = askStatus(org, selectedId, botId);
	return status === "self" || status === "none" ? null : status;
}

export function toOrgBot(bot: IBot): IOrgBot {
	return {
		id: bot.id,
		owner_id: bot.owner_id,
		system: bot.system,
		name: bot.name,
		description: bot.description,
		appearance: bot.appearance,
		location: bot.location,
		classification: bot.classification?.classification ?? null,
	};
}

export function isFlux(bot: Pick<IOrgBot, "system">): boolean {
	return bot.system === "flux";
}

export function benchBots(
	bots: readonly IBot[] | undefined,
	org: IOrgView,
	me: string | null,
): IBot[] {
	if (!me) return [];
	return (bots ?? []).filter(
		(bot) => bot.owner_id === me && !isFlux(bot) && !activeTeamOf(org, bot.id),
	);
}

export type MoveMode = "move" | "invite";

export interface MoveTarget {
	team: ITeam;
	mode: MoveMode;
}

export function moveMode(
	team: ITeam,
	bot: Pick<IOrgBot, "id" | "owner_id" | "system">,
	me: string | null,
): MoveMode | null {
	if (!me || isFlux(bot)) return null;
	if (team.bots.some((entry) => entry.bot_id === bot.id)) return null;
	if (bot.owner_id === me) {
		return isTeamMember(team, me) ? "move" : null;
	}
	return isTeamAdmin(team, me) ? "invite" : null;
}

export function moveTargets(
	org: IOrgView,
	bot: Pick<IOrgBot, "id" | "owner_id" | "system">,
	me: string | null,
): MoveTarget[] {
	return org.teams
		.flatMap((team) => {
			const mode = moveMode(team, bot, me);
			return mode ? [{ team, mode }] : [];
		})
		.sort((a, b) => a.team.name.localeCompare(b.team.name));
}

export function canTakeOut(
	team: ITeam,
	bot: Pick<IOrgBot, "owner_id">,
	me: string | null,
): boolean {
	return Boolean(me) && (bot.owner_id === me || isTeamAdmin(team, me));
}

export function descendantIds(org: IOrgView, teamId: string): Set<string> {
	const ids = new Set<string>();
	const stack = [teamId];
	while (stack.length > 0) {
		const current = stack.pop() as string;
		for (const team of org.teams) {
			if (team.parent_id === current && !ids.has(team.id)) {
				ids.add(team.id);
				stack.push(team.id);
			}
		}
	}
	return ids;
}

export function partOfOptions(
	org: IOrgView,
	team: ITeam,
	me: string | null,
): ITeam[] {
	const blocked = descendantIds(org, team.id);
	blocked.add(team.id);
	return org.teams
		.filter((entry) => !blocked.has(entry.id) && isTeamAdmin(entry, me))
		.sort((a, b) => a.name.localeCompare(b.name));
}

export function newTeamParents(org: IOrgView, me: string | null): ITeam[] {
	return org.teams
		.filter((team) => isTeamAdmin(team, me))
		.sort((a, b) => a.name.localeCompare(b.name));
}

export type DropAction = "add" | "invite";

export function dropAction(
	team: ITeam,
	bot: Pick<IOrgBot, "id" | "owner_id" | "system">,
	me: string | null,
): DropAction | null {
	const mode = moveMode(team, bot, me);
	if (mode === "move") return "add";
	return mode === "invite" ? "invite" : null;
}

export function waitingOwners(team: ITeam): string[] {
	return [
		...new Set(
			team.bots
				.filter((bot) => bot.status === "active" && !bot.direct_agreed)
				.map((bot) => bot.owner_id),
		),
	];
}

export function isWaitingForConsent(team: ITeam): boolean {
	return team.direct_asks && waitingOwners(team).length > 0;
}

export interface OrgPerson {
	userId: string;
	teams: string[];
}

/**
 * Active people of the team's org who are not in the team yet, in org order: the likeliest people to
 * invite. Their teams leave out the org's top team unless it is their only one.
 */
export function orgPeopleOutside(
	org: IOrgView,
	team: ITeam,
	me: string | null,
): OrgPerson[] {
	const root = rootTeamId(org, team.id);
	const rootName = root ? (teamById(org, root)?.name ?? "") : "";
	const inTeam = new Set(team.people.map((person) => person.user_id));
	const found = new Map<string, OrgPerson>();
	for (const entry of flattenForest(orgForest(org))) {
		if (rootTeamId(org, entry.team.id) !== root) continue;
		for (const person of entry.team.people) {
			if (
				person.status !== "active" ||
				person.user_id === me ||
				inTeam.has(person.user_id)
			)
				continue;
			const known = found.get(person.user_id) ?? {
				userId: person.user_id,
				teams: [],
			};
			if (entry.team.id !== root) known.teams.push(entry.team.name);
			found.set(person.user_id, known);
		}
	}
	return [...found.values()].map((person) =>
		person.teams.length > 0 ? person : { ...person, teams: [rootName] },
	);
}

export function teamCounts(team: ITeam) {
	return {
		bots: activeBots(team).length,
		people: team.people.filter((person) => person.status === "active").length,
	};
}

export function pendingTeamName(
	org: IOrgView,
	botId: string,
): string | undefined {
	return pendingTeamsOf(org, botId)[0]?.name;
}

export function isBotOwner(
	org: IOrgView,
	botId: string,
	me: string | null,
): boolean {
	return Boolean(me) && orgBot(org, botId)?.owner_id === me;
}

export function leadChain(org: IOrgView, botId: string): IOrgBot[] {
	return reportingLine(org, botId).flatMap((id) => {
		const bot = orgBot(org, id);
		return bot ? [bot] : [];
	});
}

export function parentTeamName(org: IOrgView, team: ITeam): string | null {
	return team.parent_id ? (teamById(org, team.parent_id)?.name ?? null) : null;
}

export const TEAM_TONES = [
	"info",
	"good",
	"warning",
	"locked",
	"paused",
] as const;
export type TeamTone = (typeof TEAM_TONES)[number];
export const TEAM_ICONS = [
	"briefcase",
	"shield",
	"headset",
	"users",
	"landmark",
	"flask",
	"megaphone",
	"wrench",
] as const;
export type TeamIconKey = (typeof TEAM_ICONS)[number];

function hashId(id: string): number {
	let hash = 0;
	for (let index = 0; index < id.length; index += 1) {
		hash = (hash * 31 + id.charCodeAt(index)) >>> 0;
	}
	return hash;
}

const NAMED_ICONS: ReadonlyArray<[RegExp, TeamIconKey]> = [
	[/\b(security|secops|safety|threat|risk|privacy)\b/i, "shield"],
	[/\b(people|hr|human|talent|recruiting|hiring|staff)\b/i, "users"],
	[
		/\b(finance|financial|accounting|budget|payroll|treasury|legal|compliance)\b/i,
		"landmark",
	],
	[
		/\b(it|ops|operations|infra|infrastructure|devops|platform|engineering)\b/i,
		"wrench",
	],
	[/\b(support|helpdesk|service|customer|success)\b/i, "headset"],
	[/\b(marketing|sales|growth|comms|communications|pr|brand)\b/i, "megaphone"],
	[/\b(research|lab|science|data|analytics|r&d)\b/i, "flask"],
];

/** A team's tile: an icon its name suggests, else one picked from its id, and a tone from its id. */
export function teamTile(
	teamId: string,
	name = "",
): {
	tone: TeamTone;
	icon: TeamIconKey;
} {
	const hash = hashId(teamId);
	const named = NAMED_ICONS.find(([pattern]) => pattern.test(name))?.[1];
	return {
		tone: TEAM_TONES[hash % TEAM_TONES.length],
		icon:
			named ??
			TEAM_ICONS[Math.floor(hash / TEAM_TONES.length) % TEAM_ICONS.length],
	};
}

export interface BriefingLine {
	kind: "item" | "text";
	text: string;
	/** Nesting of an item: two spaces of indent per level, as the org outline writes it. */
	depth: number;
}

export interface BriefingSection {
	heading: string | null;
	lines: BriefingLine[];
}

export function parseBriefing(text: string): BriefingSection[] {
	const sections: BriefingSection[] = [];
	let current: BriefingSection | null = null;
	for (const raw of text.split("\n")) {
		const line = raw.trimEnd();
		if (line.trim() === "") continue;
		if (line.startsWith("## ")) {
			current = { heading: line.slice(3).trim(), lines: [] };
			sections.push(current);
			continue;
		}
		if (!current) {
			current = { heading: null, lines: [] };
			sections.push(current);
		}
		const item = /^(\s*)[-*]\s+(.*)$/.exec(line);
		current.lines.push(
			item
				? {
						kind: "item",
						text: item[2],
						depth: Math.floor(item[1].length / 2),
					}
				: { kind: "text", text: line.trim(), depth: 0 },
		);
	}
	return sections;
}

export function previewBriefing(
	sections: BriefingSection[],
	maxLines: number,
): BriefingSection[] {
	let left = maxLines;
	const out: BriefingSection[] = [];
	for (const section of sections) {
		if (left <= 0) break;
		const lines = section.lines.slice(0, left);
		left -= lines.length + (section.heading ? 1 : 0);
		out.push({ ...section, lines });
	}
	return out;
}

export function briefingLineCount(sections: BriefingSection[]): number {
	return sections.reduce(
		(sum, section) => sum + section.lines.length + (section.heading ? 1 : 0),
		0,
	);
}
