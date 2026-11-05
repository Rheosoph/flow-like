import type { IBot } from "../../packages/ui/state/backend-state/bot-state";
import type {
	IConversation,
	IInvitation,
} from "../../packages/ui/state/backend-state/inbox-state";
import type { IUserLookup } from "../../packages/ui/state/backend-state/types";

/** The returning user's sub; `react-oidc-context` is not mocked, so the fixture reads it from auth. */
export const FIXTURE_ME = "fixture-user";

const minutesAgo = (minutes: number) =>
	new Date(Date.now() - minutes * 60_000).toISOString();

function bot(id: string, overrides: Partial<IBot>): IBot {
	return {
		id,
		owner_id: FIXTURE_ME,
		system: null,
		preset_id: null,
		name: null,
		description: null,
		instructions: null,
		appearance: { kind: "pip" } as IBot["appearance"],
		model: { primary: null, fallbacks: [] },
		classification: {} as IBot["classification"],
		capabilities: [],
		profile_id: null,
		location: "server",
		device_id: null,
		created_at: minutesAgo(60 * 24 * 30),
		updated_at: minutesAgo(60),
		...overrides,
	};
}

export const fixtureBots = (me: string): IBot[] =>
	[
		bot("bot_general", {
			system: "general",
			appearance: { kind: "pip" } as IBot["appearance"],
		}),
		bot("bot_flux", {
			system: "flux",
			appearance: { kind: "flux" } as IBot["appearance"],
		}),
		bot("bot_scout", {
			name: "Scout",
			description: "Research digest, weekdays at 7:00",
			appearance: { kind: "scout" } as IBot["appearance"],
			capabilities: ["internet"],
		}),
		bot("bot_secretary", {
			name: "Secretary",
			description: "Answers mail and books meetings for you",
			appearance: { kind: "mochi" } as IBot["appearance"],
			capabilities: ["encryption"],
		}),
		bot("bot_ledger", {
			name: "Ledger",
			description: "Reconciles invoices on the Mac mini",
			appearance: { kind: "pixel" } as IBot["appearance"],
			location: "desktop",
		}),
	].map((item) => ({ ...item, owner_id: me }));

const fixtureUsers: Record<string, IUserLookup> = {
	anna: { id: "anna", name: "Anna Weber", created_at: "" },
	jonas: { id: "jonas", name: "Jonas Keller", created_at: "" },
	maria: { id: "maria", name: "Maria Lopez", created_at: "" },
	bot_triage: {
		id: "bot_triage",
		name: "Triage",
		created_at: "",
		bot: {
			owner_id: "maria",
			system: null,
			appearance: { kind: "beacon" } as IBot["appearance"],
			deleted: false,
		},
	},
};

/** What `/user/lookup` answers: people by name, bots with their owner and look. */
export function fixtureLookup(me: string, id: string): IUserLookup {
	const own = fixtureBots(me).find((item) => item.id === id);
	if (own)
		return {
			id,
			name: own.name ?? undefined,
			created_at: own.created_at,
			bot: {
				owner_id: me,
				system: own.system,
				appearance: own.appearance,
				deleted: false,
			},
		};
	return fixtureUsers[id] ?? { id, created_at: "" };
}

function member(principal_id: string, kind: "user" | "bot" = "user") {
	return { principal_id, kind, added_by: null, joined_at: minutesAgo(9_000) };
}

function message(
	conversationId: string,
	senderId: string,
	text: string,
	minutes: number,
): NonNullable<IConversation["last_entry"]> {
	return {
		id: `${conversationId}-last`,
		message_id: `${conversationId}-message`,
		conversation_id: conversationId,
		sender_id: senderId,
		kind: "message",
		envelope: { v: 1, enc: "none", body: { text } },
		created_at: minutesAgo(minutes),
		cursor: `${minutes}:${conversationId}`,
	};
}

function conversation(
	id: string,
	kind: IConversation["kind"],
	members: IConversation["members"],
	last: IConversation["last_entry"],
	overrides: Partial<IConversation> = {},
): IConversation {
	return {
		id,
		kind,
		title: null,
		encryption: "plain",
		created_by: members[0]?.principal_id ?? "",
		created_at: minutesAgo(9_000),
		last_activity_at: last?.created_at ?? minutesAgo(9_000),
		members,
		unread_count: 0,
		last_entry: last,
		...overrides,
	};
}

export const fixtureConversations = (me: string): IConversation[] => [
	conversation(
		"room-launch",
		"group",
		[member(me), member("anna"), member("jonas"), member("bot_scout", "bot")],
		message(
			"room-launch",
			"bot_scout",
			"Three competitor launches this week. Summary is pinned.",
			6,
		),
		{ title: "Q4 launch", unread_count: 3 },
	),
	conversation(
		"dm-secretary",
		"direct",
		[member(me), member("bot_secretary", "bot")],
		message(
			"dm-secretary",
			"bot_secretary",
			"I drafted replies to four emails. Want to review them before I send?",
			14,
		),
		{ unread_count: 1 },
	),
	conversation(
		"dm-anna",
		"direct",
		[member(me), member("anna")],
		message(
			"dm-anna",
			"anna",
			"Can you look at the onboarding flow before Friday?",
			42,
		),
		{ unread_count: 1 },
	),
	conversation(
		"room-support",
		"group",
		[
			member(me),
			member("maria"),
			member("bot_triage", "bot"),
			member("bot_general", "bot"),
		],
		message(
			"room-support",
			"maria",
			"Triage cleared the morning queue. Nice work everyone.",
			95,
		),
		{ title: "Support desk" },
	),
	conversation(
		"dm-scout",
		"direct",
		[member(me), member("bot_scout", "bot")],
		message("dm-scout", me, "Add the pricing pages to tomorrow's digest.", 180),
	),
	conversation(
		"room-design",
		"group",
		[member(me), member("jonas"), member("anna"), member("maria")],
		message(
			"room-design",
			"jonas",
			"New mockups for the settings page are up.",
			60 * 26,
		),
		{ title: "Design crit" },
	),
];

export const fixtureInvitations = (me: string): IInvitation[] => [
	{
		id: `room-roadmap:${me}`,
		conversation_id: "room-roadmap",
		title: "2027 roadmap",
		principal_id: me,
		kind: "user",
		bot_name: null,
		invited_by: "jonas",
		invited_at: minutesAgo(25),
		conversation_kind: "group",
	},
];
