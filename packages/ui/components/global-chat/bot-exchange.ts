export type BotExchangeState =
	| "answered"
	| "pending"
	| "waiting"
	| "denied"
	| "error";

export interface BotExchangeView {
	state: BotExchangeState;
	callerName: string;
	calleeId: string;
	calleeName: string;
	question: string;
	answer: string | null;
	conversationId: string | null;
	note: string | null;
}

const STATES: Readonly<Record<string, BotExchangeState>> = {
	answered: "answered",
	pending: "pending",
	parked: "waiting",
	denied: "denied",
};

function asRecord(value: unknown): Record<string, unknown> | null {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: null;
}

function text(value: unknown): string | null {
	return typeof value === "string" && value.trim() ? value : null;
}

/** The ask_bot tool result as the exchange card shows it; null for any other tool result. */
export function parseAskBotResult(
	raw: unknown,
	callerName: string,
): BotExchangeView | null {
	let value = raw;
	if (typeof raw === "string") {
		try {
			value = JSON.parse(raw);
		} catch {
			return null;
		}
	}
	const result = asRecord(value);
	const exchange = asRecord(result?.exchange);
	const callee = asRecord(exchange?.callee);
	if (!result || result.tool !== "ask_bot" || !exchange || !callee) return null;
	const state = STATES[String(exchange.status)] ?? "error";
	return {
		state,
		callerName,
		calleeId: text(callee.id) ?? "",
		calleeName: text(callee.name) ?? text(callee.id) ?? "",
		question: text(exchange.question) ?? "",
		answer: text(exchange.answer),
		conversationId: text(exchange.conversation_id),
		note: state === "answered" ? null : text(result.message),
	};
}

export interface ExchangeConversationInput {
	id: string;
	kind: string;
	title: string | null;
	last_activity_at: string;
	members: ReadonlyArray<{ principal_id: string; kind: string }>;
}

export interface BotExchangeEntry {
	id: string;
	title: string;
	otherBotId: string;
	lastActivityAt: string;
}

/** The conversations in which `botId` talks with another of its owner's bots, newest first. */
export function exchangesOfBot(
	conversations: ReadonlyArray<ExchangeConversationInput>,
	botId: string,
): BotExchangeEntry[] {
	return conversations
		.filter((conversation) => conversation.kind === "group")
		.flatMap((conversation) => {
			const bots = conversation.members.filter(
				(member) => member.kind === "bot",
			);
			const other = bots.find((member) => member.principal_id !== botId);
			const involved = bots.some((member) => member.principal_id === botId);
			return involved && other && bots.length === 2
				? [
						{
							id: conversation.id,
							title: conversation.title ?? other.principal_id,
							otherBotId: other.principal_id,
							lastActivityAt: conversation.last_activity_at,
						},
					]
				: [];
		})
		.sort((a, b) => b.lastActivityAt.localeCompare(a.lastActivityAt));
}
