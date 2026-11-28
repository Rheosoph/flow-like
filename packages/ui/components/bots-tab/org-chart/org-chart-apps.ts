import {
	activeBots,
	activeTeamOf,
	isTeamAdmin,
	isTeamMember,
	orgBot,
} from "../../../lib/bot-teams/org-model";
import type {
	IOrgApp,
	IOrgAppStats,
	IOrgView,
	ITeam,
	ITeamApp,
	OrgAppType,
} from "../../../state/backend-state/bot-team-state";

/** How the chart groups app types, in the order the toolbar filter and the deck use. */
export const APP_FAMILIES = ["agents", "apps", "data", "automations"] as const;
export type AppFamily = (typeof APP_FAMILIES)[number] | "other";
export type AppFilter = "all" | (typeof APP_FAMILIES)[number];

const FAMILY_ORDER: readonly AppFamily[] = [...APP_FAMILIES, "other"];

export function appFamily(type: OrgAppType | null | undefined): AppFamily {
	switch (type) {
		case "Agent":
			return "agents";
		case "CustomInterface":
		case "Form":
			return "apps";
		case "DataFocus":
		case "Analytics":
			return "data";
		case "DataPipeline":
			return "automations";
		default:
			return "other";
	}
}

export function matchesFilter(
	app: Pick<IOrgApp, "app_type">,
	filter: AppFilter,
): boolean {
	return filter === "all" || appFamily(app.app_type) === filter;
}

export function orgApp(org: IOrgView, appId: string): IOrgApp | undefined {
	return org.apps?.find((app) => app.id === appId);
}

/** Bots of the org that may use the app. */
export function appUsers(org: IOrgView, appId: string): string[] {
	return org.bots
		.filter((bot) => bot.app_ids?.includes(appId))
		.map((bot) => bot.id);
}

export interface DeckEntry {
	app: IOrgApp;
	/** Its pin in this team; null for an app only the team's bots use. */
	pin: ITeamApp | null;
	/** Active bots of the team that may use it. */
	usedBy: string[];
}

function deckOrder(a: DeckEntry, b: DeckEntry): number {
	const family =
		FAMILY_ORDER.indexOf(appFamily(a.app.app_type)) -
		FAMILY_ORDER.indexOf(appFamily(b.app.app_type));
	if (family !== 0) return family;
	const pinned = Number(b.pin !== null) - Number(a.pin !== null);
	return pinned !== 0 ? pinned : a.app.name.localeCompare(b.app.name);
}

/** The apps a team's deck shows: its pins, then apps only its active bots use, grouped by family. */
export function teamDeck(org: IOrgView, team: ITeam): DeckEntry[] {
	const botIds = activeBots(team).map((entry) => entry.bot_id);
	const usersOf = (appId: string) =>
		botIds.filter((id) => orgBot(org, id)?.app_ids?.includes(appId));
	const entries: DeckEntry[] = [];
	const seen = new Set<string>();
	for (const pin of team.apps ?? []) {
		const app = orgApp(org, pin.app_id);
		if (!app || seen.has(app.id)) continue;
		seen.add(app.id);
		entries.push({ app, pin, usedBy: usersOf(app.id) });
	}
	for (const botId of botIds) {
		for (const appId of orgBot(org, botId)?.app_ids ?? []) {
			const app = orgApp(org, appId);
			if (!app || seen.has(appId)) continue;
			seen.add(appId);
			entries.push({ app, pin: null, usedBy: usersOf(appId) });
		}
	}
	return entries.sort(deckOrder);
}

/** Teams the app is actively pinned to. */
export function pinnedTeams(org: IOrgView, appId: string): ITeam[] {
	return org.teams.filter((team) =>
		(team.apps ?? []).some(
			(pin) => pin.app_id === appId && pin.status === "active",
		),
	);
}

/** Pins of the app the reader sees, active and pending, with their teams. */
export function appPins(
	org: IOrgView,
	appId: string,
): { team: ITeam; pin: ITeamApp }[] {
	return org.teams.flatMap((team) =>
		(team.apps ?? [])
			.filter((pin) => pin.app_id === appId)
			.map((pin) => ({ team, pin })),
	);
}

export function canPinTo(team: ITeam, me: string | null): boolean {
	return isTeamMember(team, me);
}

/** Team admins, the pinner and the app's admins may take a pin off. */
export function canUnpin(
	team: ITeam,
	pin: ITeamApp,
	app: Pick<IOrgApp, "reader_admin">,
	me: string | null,
): boolean {
	return (
		Boolean(me) &&
		(isTeamAdmin(team, me) || pin.pinned_by === me || app.reader_admin)
	);
}

export interface AppLinks {
	botIds: ReadonlySet<string>;
	teamIds: ReadonlySet<string>;
}

/** What lights up when an app is selected: the bots that use it and the teams it belongs to. */
export function appLinks(org: IOrgView, appId: string): AppLinks {
	const botIds = new Set(appUsers(org, appId));
	const teamIds = new Set(pinnedTeams(org, appId).map((team) => team.id));
	for (const botId of botIds) {
		const team = activeTeamOf(org, botId);
		if (team) teamIds.add(team.id);
	}
	return { botIds, teamIds };
}

export function sumRecent(values: readonly number[], days: number): number {
	return values.slice(-days).reduce((sum, value) => sum + value, 0);
}

export interface Activity {
	/** Runs over the last 7 days. */
	week: number;
	/** Failed runs today. */
	failedToday: number;
	/** Failed runs over the last 7 days. */
	failedWeek: number;
}

export function activityOf(stats: IOrgAppStats): Activity {
	return {
		week: sumRecent(stats.runs, 7),
		failedToday: stats.failed.at(-1) ?? 0,
		failedWeek: sumRecent(stats.failed, 7),
	};
}

export interface RunBar {
	runs: number;
	failed: number;
	/** 0..1 of the busiest day. */
	height: number;
}

export function runBars(stats: IOrgAppStats): RunBar[] {
	const peak = Math.max(1, ...stats.runs);
	return stats.runs.map((runs, index) => ({
		runs,
		failed: stats.failed[index] ?? 0,
		height: runs / peak,
	}));
}

export type Schedule =
	| { kind: "minutes"; every: number }
	| { kind: "hourly" }
	| { kind: "hours"; every: number }
	| { kind: "daily"; time: string }
	| { kind: "cron"; expression: string };

const NUMBER = /^\d+$/;
const STEP = /^\*\/(\d+)$/;

function clock(hour: string, minute: string): string {
	return `${hour.padStart(2, "0")}:${minute.padStart(2, "0")}`;
}

/** A cron expression in words where it has a common shape; a leading seconds field is ignored. */
export function scheduleOf(expression: string): Schedule {
	const fields = expression.trim().split(/\s+/);
	const [minute, hour, day, month, weekday] =
		fields.length >= 6 ? fields.slice(1) : fields;
	const raw: Schedule = { kind: "cron", expression: expression.trim() };
	if (!minute || !hour || day !== "*" || month !== "*") return raw;
	if (weekday !== "*" && weekday !== "?") return raw;
	const minuteStep = STEP.exec(minute);
	if (minuteStep && hour === "*") {
		return { kind: "minutes", every: Number(minuteStep[1]) };
	}
	if (NUMBER.test(minute) && hour === "*") return { kind: "hourly" };
	const hourStep = STEP.exec(hour);
	if (NUMBER.test(minute) && hourStep) {
		return { kind: "hours", every: Number(hourStep[1]) };
	}
	if (NUMBER.test(minute) && NUMBER.test(hour)) {
		return { kind: "daily", time: clock(hour, minute) };
	}
	return raw;
}

export type TriggerKind = "chat" | "mail" | "schedule" | "webhook" | "form";

const TRIGGER_KINDS: Record<string, TriggerKind> = {
	simple_chat: "chat",
	chat: "chat",
	discord: "chat",
	telegram: "chat",
	slack: "chat",
	email: "mail",
	cron: "schedule",
	api: "webhook",
	rest: "webhook",
	http: "webhook",
	webhook: "webhook",
	mcp: "webhook",
	generic_form: "form",
};

/** The kinds of what starts an app, each once, in a stable order. */
export function triggerKinds(triggers: readonly string[]): TriggerKind[] {
	const kinds = new Set(
		triggers.flatMap((trigger) => {
			const kind = TRIGGER_KINDS[trigger];
			return kind ? [kind] : [];
		}),
	);
	return (["chat", "mail", "form", "schedule", "webhook"] as const).filter(
		(kind) => kinds.has(kind),
	);
}

/** The two halves of the bus wire above one cell of the deck, left and right of its center. */
export interface BusHalves {
	left: boolean;
	right: boolean;
}

/**
 * Which halves of the deck's bus lie on the way from a lit cell to the source the bus hangs from.
 * Cells are one unit wide; `source` is where the line from the team meets the bus, in units from
 * the left edge.
 */
export function litBus(lit: readonly boolean[], source: number): BusHalves[] {
	const spans = lit.flatMap((on, index) => {
		if (!on) return [];
		const center = index + 0.5;
		return [[Math.min(center, source), Math.max(center, source)] as const];
	});
	const covered = (from: number, to: number) =>
		spans.some(([start, end]) => start <= from && end >= to);
	return lit.map((_, index) => ({
		left: covered(index, index + 0.5),
		right: covered(index + 0.5, index + 1),
	}));
}
