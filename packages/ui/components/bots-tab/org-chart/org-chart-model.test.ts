import { describe, expect, it } from "bun:test";
import type { IBot } from "../../../state/backend-state/bot-state";
import type {
	IOrgBot,
	IOrgView,
	ITeam,
} from "../../../state/backend-state/bot-team-state";
import {
	benchBots,
	canTakeOut,
	dropAction,
	groupAsks,
	highlightFor,
	isWaitingForConsent,
	moveTargets,
	orgPeopleOutside,
	parseBriefing,
	partOfOptions,
	previewBriefing,
	teamTile,
} from "./org-chart-model";

const appearance = {} as IOrgBot["appearance"];

function bot(id: string, owner: string, extra: Partial<IOrgBot> = {}): IOrgBot {
	return {
		id,
		owner_id: owner,
		name: id,
		appearance,
		location: "server",
		classification: "internal",
		...extra,
	};
}

function team(id: string, extra: Partial<ITeam> = {}): ITeam {
	return {
		id,
		name: id,
		direct_asks: false,
		people: [],
		bots: [],
		created_at: "",
		updated_at: "",
		...extra,
	};
}

const member = (user_id: string, role: "admin" | "member" = "member") => ({
	user_id,
	role,
	status: "active" as const,
});
const inTeam = (bot_id: string, owner_id: string, extra = {}) => ({
	bot_id,
	owner_id,
	status: "active" as const,
	direct_agreed: false,
	...extra,
});

const org: IOrgView = {
	teams: [
		team("kestrel", {
			lead_bot_id: "boss",
			people: [member("mara", "admin"), member("jonas")],
			bots: [inTeam("boss", "mara")],
		}),
		team("sec", {
			parent_id: "kestrel",
			lead_bot_id: "sentry",
			people: [member("mara", "admin"), member("jonas")],
			bots: [inTeam("sentry", "jonas"), inTeam("scout", "mara")],
		}),
		team("fin", {
			parent_id: "kestrel",
			people: [member("mara", "admin")],
			bots: [
				inTeam("ledger", "anna"),
				inTeam("late", "anna", { status: "pending" }),
			],
		}),
	],
	bots: [
		bot("boss", "mara"),
		bot("sentry", "jonas"),
		bot("scout", "mara"),
		bot("ledger", "anna"),
		bot("late", "anna"),
		bot("secret", "anna", { classification: "restricted" }),
	],
	invitations: [],
};

describe("highlightFor", () => {
	it("walks leads and the team path", () => {
		const hl = highlightFor(org, "scout");
		expect([...hl.botIds]).toEqual(["scout", "sentry", "boss"]);
		expect([...hl.teamIds].sort()).toEqual(["kestrel", "sec"]);
	});
	it("is empty without a selection", () => {
		expect(highlightFor(org, null).botIds.size).toBe(0);
	});
});

describe("groupAsks", () => {
	it("sorts others by status", () => {
		const groups = groupAsks(org, "scout");
		expect(groups.ok.map((b) => b.id)).toContain("sentry");
		expect(groups.later.map((b) => b.id)).toEqual(["late"]);
		expect(groups.blocked.map((b) => b.id)).toEqual(["secret"]);
	});
});

describe("benchBots", () => {
	it("lists own bots outside teams without Flux", () => {
		const own = [
			{ id: "a", owner_id: "mara", system: null },
			{ id: "boss", owner_id: "mara", system: null },
			{ id: "flux", owner_id: "mara", system: "flux" },
			{ id: "x", owner_id: "jonas", system: null },
		] as IBot[];
		expect(benchBots(own, org, "mara").map((b) => b.id)).toEqual(["a"]);
	});
});

describe("move rules", () => {
	it("own bots move into teams you belong to", () => {
		const targets = moveTargets(org, bot("scout", "mara"), "mara");
		expect(targets.map((t) => [t.team.id, t.mode])).toEqual([
			["fin", "move"],
			["kestrel", "move"],
		]);
	});
	it("other bots are invited by admins only", () => {
		const ledger = bot("ledger", "anna");
		expect(moveTargets(org, ledger, "jonas")).toEqual([]);
		expect(moveTargets(org, ledger, "mara").map((t) => t.mode)).toEqual([
			"invite",
			"invite",
		]);
	});
	it("Flux never moves", () => {
		expect(
			moveTargets(org, bot("f", "mara", { system: "flux" }), "mara"),
		).toEqual([]);
	});
	it("drops map to add or invite", () => {
		expect(dropAction(org.teams[0], bot("z", "mara"), "mara")).toBe("add");
		expect(dropAction(org.teams[0], bot("z", "anna"), "mara")).toBe("invite");
		expect(dropAction(org.teams[0], bot("z", "anna"), "jonas")).toBeNull();
	});
	it("owner or admin takes a bot out", () => {
		expect(canTakeOut(org.teams[1], bot("sentry", "jonas"), "jonas")).toBe(
			true,
		);
		expect(canTakeOut(org.teams[1], bot("sentry", "jonas"), "mara")).toBe(true);
		expect(canTakeOut(org.teams[1], bot("scout", "mara"), "jonas")).toBe(false);
	});
});

describe("partOfOptions", () => {
	it("excludes the team and its descendants", () => {
		expect(partOfOptions(org, org.teams[0], "mara")).toEqual([]);
		expect(partOfOptions(org, org.teams[1], "mara").map((t) => t.id)).toEqual([
			"fin",
			"kestrel",
		]);
	});
});

describe("consent", () => {
	it("waits while an active bot has not agreed", () => {
		const t = team("t", { direct_asks: true, bots: [inTeam("a", "mara")] });
		expect(isWaitingForConsent(t)).toBe(true);
		expect(
			isWaitingForConsent({
				...t,
				bots: [inTeam("a", "mara", { direct_agreed: true })],
			}),
		).toBe(false);
	});
});

describe("briefing", () => {
	const sections = parseBriefing(
		"## YOUR TEAM\nYou are in X.\n- a\n- b\n\n## OTHER\n- c",
	);
	it("parses headings and items", () => {
		expect(sections).toHaveLength(2);
		expect(sections[0].lines.map((l) => l.kind)).toEqual([
			"text",
			"item",
			"item",
		]);
	});
	it("previews by line budget", () => {
		expect(previewBriefing(sections, 3)).toHaveLength(1);
	});
	it("keeps the org outline's nesting", () => {
		const [org] = parseBriefing(
			"## THE ORGANIZATION\n- Kestrel — lead Atlas\n  - Security — lead Sentinel\n    - Patch, Jonas's bot",
		);
		expect(org.lines.map((line) => line.depth)).toEqual([0, 1, 2]);
	});
});

describe("teamTile", () => {
	it("is stable per id", () => {
		expect(teamTile("abc")).toEqual(teamTile("abc"));
	});
});

describe("orgPeopleOutside", () => {
	const withOthers: IOrgView = {
		...org,
		teams: [
			...org.teams.map((entry) =>
				entry.id === "kestrel"
					? { ...entry, people: [...entry.people, member("lena")] }
					: entry,
			),
			team("ops", {
				parent_id: "kestrel",
				people: [
					member("ilse"),
					{ user_id: "tomas", role: "member", status: "pending" },
				],
			}),
			team("elsewhere", { people: [member("zoe", "admin")] }),
		],
	};
	it("suggests active people of the same org who are not in the team", () => {
		const fin = withOthers.teams.find((entry) => entry.id === "fin") as ITeam;
		expect(orgPeopleOutside(withOthers, fin, "mara")).toEqual([
			{ userId: "jonas", teams: ["sec"] },
			{ userId: "lena", teams: ["kestrel"] },
			{ userId: "ilse", teams: ["ops"] },
		]);
	});
	it("leaves out people already in the team and other orgs", () => {
		const sec = withOthers.teams.find((entry) => entry.id === "sec") as ITeam;
		expect(
			orgPeopleOutside(withOthers, sec, "mara").map((person) => person.userId),
		).toEqual(["lena", "ilse"]);
	});
});
