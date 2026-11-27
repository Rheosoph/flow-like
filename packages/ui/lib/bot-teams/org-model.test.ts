import { describe, expect, test } from "bun:test";
import type {
	IOrgBot,
	IOrgView,
	ITeam,
	ITeamBot,
} from "../../state/backend-state/bot-team-state";
import {
	activeTeamOf,
	askStatus,
	classificationOf,
	flattenForest,
	isGatedInRoom,
	isTeamAdmin,
	isTeamConversation,
	leadOfferFor,
	orgForest,
	pendingTeamsOf,
	reachOf,
	reportingLine,
	teamPath,
	teamTag,
} from "./org-model";

function bot(id: string, owner: string, classification?: string): IOrgBot {
	return {
		id,
		owner_id: owner,
		appearance: { kind: "robot" },
		location: "server",
		classification,
	};
}

function member(
	id: string,
	owner: string,
	over: Partial<ITeamBot> = {},
): ITeamBot {
	return {
		bot_id: id,
		owner_id: owner,
		status: "active",
		direct_agreed: false,
		...over,
	};
}

function team(id: string, over: Partial<ITeam> = {}): ITeam {
	return {
		id,
		name: id,
		direct_asks: false,
		people: [],
		bots: [],
		created_at: "",
		updated_at: "",
		...over,
	};
}

const org: IOrgView = {
	invitations: [],
	bots: [
		bot("kestrel-lead", "mara"),
		bot("sec-lead", "mara"),
		bot("sec-a", "jonas"),
		bot("sec-b", "anna"),
		bot("fin-lead", "ilse"),
		bot("fin-a", "tomas"),
		bot("mara-solo", "mara"),
		bot("secret", "mara", "restricted"),
		bot("pending", "anna"),
		bot("outsider", "zed"),
	],
	teams: [
		team("Kestrel", {
			lead_bot_id: "kestrel-lead",
			bots: [member("kestrel-lead", "mara")],
		}),
		team("Security", {
			parent_id: "Kestrel",
			lead_bot_id: "sec-lead",
			direct_asks: true,
			people: [{ user_id: "mara", role: "admin", status: "active" }],
			bots: [
				member("sec-lead", "mara", { direct_agreed: true }),
				member("sec-a", "jonas", { direct_agreed: true }),
				member("sec-b", "anna"),
				member("secret", "mara", { direct_agreed: true }),
				member("pending", "anna", { status: "pending" }),
			],
		}),
		team("Finance", {
			parent_id: "Kestrel",
			lead_bot_id: "fin-lead",
			bots: [member("fin-lead", "ilse"), member("fin-a", "tomas")],
		}),
		team("Alpha"),
	],
};

describe("forest", () => {
	test("orders roots by name and nests children", () => {
		const forest = orgForest(org);
		expect(forest.map((n) => n.team.id)).toEqual(["Alpha", "Kestrel"]);
		const flat = flattenForest(forest);
		expect(flat.map((n) => [n.team.id, n.depth])).toEqual([
			["Alpha", 0],
			["Kestrel", 0],
			["Finance", 1],
			["Security", 1],
		]);
	});

	test("a dangling parent counts as a root and cycles terminate", () => {
		const odd: IOrgView = {
			...org,
			teams: [
				team("A", { parent_id: "gone" }),
				team("B", { parent_id: "C" }),
				team("C", { parent_id: "B" }),
			],
		};
		expect(orgForest(odd).map((n) => n.team.id)).toEqual(["A"]);
		expect(teamPath(odd, "B").length).toBe(2);
	});

	test("teamPath runs from the top-level team down", () => {
		expect(teamPath(org, "Security").map((t) => t.id)).toEqual([
			"Kestrel",
			"Security",
		]);
		expect(teamPath(org, "missing")).toEqual([]);
	});
});

describe("membership", () => {
	test("active and pending teams", () => {
		expect(activeTeamOf(org, "sec-a")?.id).toBe("Security");
		expect(activeTeamOf(org, "pending")).toBeUndefined();
		expect(pendingTeamsOf(org, "pending").map((t) => t.id)).toEqual([
			"Security",
		]);
	});

	test("admins need an active admin role", () => {
		const security = org.teams[1];
		expect(isTeamAdmin(security, "mara")).toBe(true);
		expect(isTeamAdmin(security, "jonas")).toBe(false);
		expect(isTeamAdmin(security, null)).toBe(false);
	});

	test("teamTag drops spaces", () => {
		expect(teamTag("IT Support")).toBe("#ITSupport");
	});
});

describe("reportingLine", () => {
	test("member reports to the team lead, then the parent lead", () => {
		expect(reportingLine(org, "sec-a")).toEqual([
			"sec-a",
			"sec-lead",
			"kestrel-lead",
		]);
	});

	test("a lead reports to the parent team's lead", () => {
		expect(reportingLine(org, "sec-lead")).toEqual([
			"sec-lead",
			"kestrel-lead",
		]);
	});

	test("a bot outside every team has only itself", () => {
		expect(reportingLine(org, "mara-solo")).toEqual(["mara-solo"]);
	});
});

describe("askStatus", () => {
	test("self", () => expect(askStatus(org, "sec-a", "sec-a")).toBe("self"));

	test("later while the owner has not accepted", () =>
		expect(askStatus(org, "sec-a", "pending")).toBe("later"));

	test("blocked by classification", () =>
		expect(askStatus(org, "sec-a", "secret")).toBe("blocked"));

	test("direct needs the switch and both agreements", () => {
		expect(askStatus(org, "sec-a", "sec-lead")).toBe("direct");
		expect(askStatus(org, "sec-a", "sec-b")).toBe("ok");
	});

	test("teammate without the switch needs the first-time OK", () =>
		expect(askStatus(org, "fin-a", "fin-lead")).toBe("ok"));

	test("own crew is ok", () =>
		expect(askStatus(org, "mara-solo", "kestrel-lead")).toBe("ok"));

	test("another team's lead is ok", () =>
		expect(askStatus(org, "sec-a", "fin-lead")).toBe("ok"));

	test("another team's member goes via its lead", () =>
		expect(askStatus(org, "sec-a", "fin-a")).toBe("via"));

	test("outside the org is none", () => {
		expect(askStatus(org, "outsider", "fin-a")).toBe("none");
		expect(askStatus(org, "sec-a", "unknown")).toBe("none");
	});
});

describe("isTeamConversation", () => {
	const org = {
		teams: [{ id: "t", name: "Security", conversation_id: "room" }],
		bots: [],
		invitations: [],
		threads: [{ team_id: "t", conversation_id: "thread" }],
	} as unknown as IOrgView;

	test("a team room and the reader's thread count, other chats do not", () => {
		expect(isTeamConversation(org, "room")).toBe(true);
		expect(isTeamConversation(org, "thread")).toBe(true);
		expect(isTeamConversation(org, "other")).toBe(false);
	});
});

describe("reach", () => {
	const days = (n: number) =>
		new Date(Date.now() + n * 86_400_000).toISOString();
	const base = (over: Partial<IOrgView> = {}): IOrgView => ({
		invitations: [],
		bots: [
			bot("lead", "owner"),
			bot("peer", "owner"),
			bot("mine", "viewer"),
			bot("top", "owner", "restricted"),
			bot("joining", "owner"),
		],
		teams: [
			team("Org", {
				lead_bot_id: "lead",
				conversation_id: "room",
				people: [{ user_id: "owner", role: "admin", status: "active" }],
				bots: [
					member("lead", "owner"),
					member("peer", "owner"),
					member("top", "owner"),
					member("mine", "viewer"),
					member("joining", "owner", { status: "pending" }),
				],
			}),
		],
		...over,
	});

	test("the hub's entry wins", () => {
		const org = base({ reach: [{ bot_id: "peer", reach: "chat" }] });
		expect(reachOf(org, "viewer", "peer").reach).toBe("chat");
	});

	test("older hubs: own, joining, classified, lead, request", () => {
		const org = base();
		expect(reachOf(org, "viewer", "mine").reach).toBe("own");
		expect(reachOf(org, "viewer", "joining").reach).toBe("unavailable");
		expect(reachOf(org, "viewer", "top").reach).toBe("classified");
		expect(reachOf(org, "viewer", "lead").reach).toBe("lead");
		expect(reachOf(org, "viewer", "peer").reach).toBe("request");
	});

	test("older hubs: a teammate reaches the team room", () => {
		const org = base();
		org.teams[0].people.push({
			user_id: "viewer",
			role: "member",
			status: "active",
		});
		const reach = reachOf(org, "viewer", "peer");
		expect(reach.reach).toBe("team_room");
		expect(reach.conversation_id).toBe("room");
	});

	test("older hubs: a raised classification opens a restricted bot", () => {
		const org = base({
			classifications: [
				{ org_id: "Org", user_id: "viewer", classification: "restricted" },
			],
		});
		expect(reachOf(org, "viewer", "top").reach).toBe("request");
	});

	test("older hubs: chats", () => {
		const chat = (
			status: "pending" | "active" | "declined",
			decided_at?: string,
		) =>
			base({
				chats: [
					{
						bot_id: "peer",
						user_id: "viewer",
						owner_id: "owner",
						status,
						conversation_id: "chat-1",
						requested_at: "",
						decided_at,
					},
				],
			});
		expect(reachOf(chat("active"), "viewer", "peer")).toMatchObject({
			reach: "chat",
			conversation_id: "chat-1",
		});
		expect(reachOf(chat("pending"), "viewer", "peer").reach).toBe("requested");
		expect(reachOf(chat("declined", days(-1)), "viewer", "peer").reach).toBe(
			"declined",
		);
		expect(reachOf(chat("declined", days(-30)), "viewer", "peer").reach).toBe(
			"request",
		);
	});
});

describe("classification, gating and lead offers", () => {
	test("people default to internal", () => {
		const org: IOrgView = {
			...base0(),
			classifications: [
				{ org_id: "o", user_id: "a", classification: "confidential" },
			],
		};
		expect(classificationOf(org, "o", "a")).toBe("confidential");
		expect(classificationOf(org, "o", "b")).toBe("internal");
		expect(classificationOf(org, "other", "a")).toBe("internal");
	});

	test("gated bots and the lead offer", () => {
		const t = team("t", {
			gated_bots: ["x"],
			lead_offer_bot_id: "x",
			bots: [member("x", "o")],
		});
		expect(isGatedInRoom(t, "x")).toBe(true);
		expect(isGatedInRoom(t, "y")).toBe(false);
		expect(leadOfferFor(t)?.bot_id).toBe("x");
		expect(leadOfferFor(team("u"))).toBeUndefined();
	});
});

function base0(): IOrgView {
	return { teams: [], bots: [], invitations: [] };
}
