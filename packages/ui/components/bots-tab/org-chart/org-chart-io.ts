import { activeBots } from "../../../lib/bot-teams/org-model";
import type {
	IOrgBot,
	ITeam,
	OrgCalendarProvider,
} from "../../../state/backend-state/bot-team-state";

export type InputPort =
	| { kind: "mail"; key: string; address: string; received: number }
	| {
			kind: "webhook";
			key: string;
			name: string;
			lastCalledAt: string | null;
	  };

export interface OutputPort {
	kind: "mail";
	key: string;
	address: string;
	sent: number;
}

export interface BotPorts {
	inputs: InputPort[];
	/** Inputs left out to keep the row short; the inspector lists them all. */
	moreInputs: number;
	outputs: OutputPort[];
}

/** Ports one bot row shows beside the team box before it folds the rest into "+N". */
export const MAX_ROW_INPUTS = 3;

const NO_PORTS: BotPorts = { inputs: [], moreInputs: 0, outputs: [] };

export function allInputs(bot: Pick<IOrgBot, "inputs">): InputPort[] {
	const inputs = bot.inputs;
	if (!inputs) return [];
	return [
		...(inputs.mail
			? [
					{
						kind: "mail" as const,
						key: "mail",
						address: inputs.mail.address,
						received: inputs.mail.received,
					},
				]
			: []),
		...(inputs.webhooks ?? []).map((hook, index) => ({
			kind: "webhook" as const,
			key: `webhook-${index}`,
			name: hook.name,
			lastCalledAt: hook.last_called_at ?? null,
		})),
	];
}

export function botPorts(
	bot: Pick<IOrgBot, "inputs">,
	max = MAX_ROW_INPUTS,
): BotPorts {
	const inputs = allInputs(bot);
	const mail = bot.inputs?.mail;
	if (inputs.length === 0 && !mail) return NO_PORTS;
	const shown = inputs.length > max ? inputs.slice(0, max - 1) : inputs;
	return {
		inputs: shown,
		moreInputs: inputs.length - shown.length,
		outputs: mail
			? [
					{
						kind: "mail",
						key: "mail-out",
						address: mail.address,
						sent: mail.sent,
					},
				]
			: [],
	};
}

export function hasPorts(bot: Pick<IOrgBot, "inputs"> | undefined): boolean {
	return Boolean(bot && allInputs(bot).length > 0);
}

export interface CalendarMark {
	/** Its own calendar: events it keeps wake it. */
	own: boolean;
	/** Connected calendars by provider, in the order it connected them. */
	connected: OrgCalendarProvider[];
}

export function calendarMark(
	bot: Pick<IOrgBot, "inputs"> | undefined,
): CalendarMark | null {
	const inputs = bot?.inputs;
	if (!inputs) return null;
	const connected = inputs.connected_calendars ?? [];
	if (!inputs.own_calendar && connected.length === 0) return null;
	return { own: inputs.own_calendar, connected };
}

export interface TeamFlow {
	/** Active bots of the team with their own mail address. */
	addresses: number;
	webhooks: number;
	received: number;
	sent: number;
	calendars: number;
}

/** What flows into and out of a team through its active bots. */
export function teamFlow(
	team: ITeam,
	botOf: (id: string) => Pick<IOrgBot, "inputs"> | undefined,
): TeamFlow {
	const flow: TeamFlow = {
		addresses: 0,
		webhooks: 0,
		received: 0,
		sent: 0,
		calendars: 0,
	};
	for (const entry of activeBots(team)) {
		const inputs = botOf(entry.bot_id)?.inputs;
		if (!inputs) continue;
		if (inputs.mail) {
			flow.addresses += 1;
			flow.received += inputs.mail.received;
			flow.sent += inputs.mail.sent;
		}
		flow.webhooks += inputs.webhooks?.length ?? 0;
		flow.calendars += inputs.connected_calendars?.length ?? 0;
	}
	return flow;
}

/** Whether any active bot of the team has a way in, so the chart gives the team its side rails. */
export function teamHasPorts(
	team: ITeam,
	botOf: (id: string) => Pick<IOrgBot, "inputs"> | undefined,
): boolean {
	return activeBots(team).some((entry) => hasPorts(botOf(entry.bot_id)));
}
