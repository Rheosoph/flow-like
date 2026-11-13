import { describe, expect, test } from "bun:test";
import {
	alwaysBlock,
	approvalItems,
	batchAnswer,
	callLine,
	callPhrase,
	callTexts,
	groupItems,
	isPastWait,
	refusalCode,
	tallyOf,
} from "./approval-batch-model";
import type { ApprovalDecision } from "./approval-entry";
import type { ConversationApprovalItem } from "./fold-entries";

const item = (
	argsHash: string,
	extra: Partial<ConversationApprovalItem> = {},
): ConversationApprovalItem => ({
	argsHash,
	tool: "web_request",
	summary: "The bot wants to send a GET request to cisa.gov.",
	scopes: ["web:cisa.gov:read"],
	calls: 1,
	afterWebContent: false,
	...extra,
});

const choices =
	(map: Record<string, ApprovalDecision>) => (call: ConversationApprovalItem) =>
		map[call.argsHash];

describe("callTexts", () => {
	const get = (hash: string, url: string, method = "GET") =>
		item(hash, {
			summary: `The bot wants to send a ${method} request to ${url}.`,
		});
	const shown = (text: { start: string; end: string }) =>
		`${text.start}${text.end}`;

	test("twelve calls that differ only at the end keep their distinguishing tails", () => {
		const items = Array.from({ length: 12 }, (_, index) =>
			get(
				`p${index + 1}`,
				`cisa.gov/news-events/cybersecurity-advisories?page=${index + 1}`,
			),
		);
		const textOf = callTexts(items);
		const texts = items.map(textOf);
		expect(texts[0]).toEqual({
			full: "Send a GET request to cisa.gov/news-events/cybersecurity-advisories?page=1",
			start: "GET /news-events/cybersecurity-advisories",
			end: "?page=1",
		});
		expect(texts[11].end).toBe("?page=12");
		expect(new Set(texts.map((text) => text.end)).size).toBe(12);
		expect(texts.every((text) => !text.full.startsWith("GET"))).toBe(true);
	});

	test("a differing last path segment stays whole", () => {
		const textOf = callTexts([
			get("a", "nvd.nist.gov/vuln/detail/CVE-2026-4410"),
			get("b", "nvd.nist.gov/vuln/detail/CVE-2026-4411"),
		]);
		expect(
			textOf(get("a", "nvd.nist.gov/vuln/detail/CVE-2026-4410")),
		).toMatchObject({
			start: "GET /vuln/detail",
			end: "/CVE-2026-4410",
		});
	});

	test("calls with nothing in common stay as they are", () => {
		const items = [
			item("a", { tool: "shell_run", summary: "Run git status" }),
			item("b", { tool: "shell_run", summary: "Open news.ycombinator.com" }),
		];
		expect(items.map(callTexts(items))).toEqual([
			{ full: "Run git status", start: "Run git status", end: "" },
			{
				full: "Open news.ycombinator.com",
				start: "Open news.ycombinator.com",
				end: "",
			},
		]);
	});

	test("a group of one keeps its full line", () => {
		const lone = get("a", "cisa.gov/news-events/alerts?p=1");
		expect(callTexts([lone])(lone)).toEqual({
			full: "Send a GET request to cisa.gov/news-events/alerts?p=1",
			start: "Send a GET request to cisa.gov/news-events/alerts?p=1",
			end: "",
		});
	});

	test("different sites keep their sites and the method", () => {
		const items = [get("a", "cisa.gov/alerts"), get("b", "nvd.nist.gov/vuln")];
		expect(items.map(callTexts(items)).map(shown)).toEqual([
			"GET cisa.gov/alerts",
			"GET nvd.nist.gov/vuln",
		]);
	});

	test("only the words every call shares at the start go", () => {
		const items = [
			item("a", {
				tool: "email_send",
				summary: "The bot wants to send the weekly digest to team@acme.com.",
			}),
			item("b", {
				tool: "email_send",
				summary: "The bot wants to send the alert digest to ops@acme.com.",
			}),
		];
		expect(items.map(callTexts(items)).map(shown)).toEqual([
			"Weekly digest to team@acme.com",
			"Alert digest to ops@acme.com",
		]);
	});

	test("a tail too long for a row loses its middle, never its end", () => {
		const long = (tail: string) =>
			get(tail, `api.example.com/v1/reports/${tail}`);
		const a = "2026-11-12-quarterly-revenue-breakdown-by-region-final";
		const b = "2026-11-13-quarterly-revenue-breakdown-by-region-final";
		const text = callTexts([long(a), long(b)])(long(a));
		expect(text.start).toBe("GET /v1/reports");
		expect(text.end.length).toBeLessThanOrEqual(36);
		expect(text.end).toContain("…");
		expect(text.end.startsWith("/2026-11-12")).toBe(true);
		expect(`/${a}`.endsWith(text.end.split("…")[1])).toBe(true);
	});
});

describe("batchAnswer", () => {
	test("sends the most common choice and lists only the calls that differ", () => {
		const items = [item("a"), item("b"), item("c"), item("d")];
		expect(
			batchAnswer(
				items,
				choices({ a: "deny", b: "deny", c: "once", d: "deny" }),
			),
		).toEqual({
			decision: "deny",
			items: [{ args_hash: "c", decision: "once" }],
		});
	});

	test("a tie goes to Once, then Deny, never Always first", () => {
		const items = [item("a"), item("b"), item("c")];
		expect(
			batchAnswer(items, choices({ a: "always", b: "deny", c: "once" }))
				.decision,
		).toBe("once");
		expect(
			batchAnswer(items.slice(0, 2), choices({ a: "always", b: "deny" }))
				.decision,
		).toBe("deny");
	});

	test("a shortcut keeps its decision and lists what it can't cover", () => {
		expect(
			batchAnswer(
				[item("a"), item("s")],
				(call) => (call.argsHash === "s" ? "deny" : "once"),
				"once",
			),
		).toEqual({
			decision: "once",
			items: [{ args_hash: "s", decision: "deny" }],
		});
	});
});

describe("groupItems", () => {
	test("groups by tool, target and flag, in the order the calls came", () => {
		const groups = groupItems([
			item("a"),
			item("b", { tool: "email_send", scopes: ["email:acme.com"] }),
			item("c", { calls: 3 }),
			item("d", { afterWebContent: true }),
		]);
		expect(
			groups.map((group) => group.items.map((call) => call.argsHash)),
		).toEqual([["a", "c"], ["b"], ["d"]]);
		expect(groups.map((group) => group.calls)).toEqual([4, 1, 1]);
		expect(groups[2].flagged).toBe(true);
	});
});

test("tallies count identical calls each and skip unanswered ones", () => {
	expect(
		tallyOf(
			[item("a", { calls: 3 }), item("b"), item("c")],
			choices({ a: "always", b: "deny" }),
		),
	).toEqual({ allowed: 3, denied: 1 });
});

test("Always is blocked after web content, for secrets and for calls without scopes", () => {
	expect(alwaysBlock(item("a"))).toBeUndefined();
	expect(alwaysBlock(item("a", { afterWebContent: true }))).toBe("webContent");
	expect(alwaysBlock(item("a", { scopes: [] }))).toBe("notRememberable");
	expect(
		alwaysBlock(item("a", { secret: { name: "API_KEY", description: "x" } })),
	).toBe("notRememberable");
	expect(alwaysBlock(item("a", { tool: "bot_update_configuration" }))).toBe(
		"notRememberable",
	);
});

test("a card for one call is its own single item", () => {
	expect(
		approvalItems({
			id: "ap-1",
			turnId: "t",
			tool: "shell_run",
			summary: "Run git",
			scopes: ["command:git"],
			afterWebContent: true,
		}),
	).toEqual([
		{
			argsHash: "ap-1",
			tool: "shell_run",
			summary: "Run git",
			scopes: ["command:git"],
			calls: 1,
			afterWebContent: true,
		},
	]);
});

test("summaries drop the core's opening and read as a line or a phrase", () => {
	const summary = "The bot wants to send a GET request to cisa.gov.";
	expect(callLine(summary)).toBe("Send a GET request to cisa.gov");
	expect(callPhrase(summary)).toBe("send a GET request to cisa.gov");
	expect(callPhrase("Run a program called git")).toBe(
		"run a program called git",
	);
	expect(callPhrase("GET cisa.gov")).toBe("GET cisa.gov");
});

test("refusal codes come from the error or its message", () => {
	expect(refusalCode({ code: "BOT_TURN_NOT_WAITING" })).toBe(
		"BOT_TURN_NOT_WAITING",
	);
	expect(
		refusalCode(
			new Error("[BOT_APPROVAL_EXPIRED; ref abc] This approval has expired"),
		),
	).toBe("BOT_APPROVAL_EXPIRED");
	expect(refusalCode(new Error("offline"))).toBeUndefined();
	expect(refusalCode("BOT_TURN_NOT_WAITING")).toBeUndefined();
});

test("a card expires seven days after it was posted", () => {
	const now = Date.UTC(2026, 10, 12);
	const day = 24 * 60 * 60 * 1000;
	expect(isPastWait(now - 7 * day + 1, now)).toBe(false);
	expect(isPastWait(now - 7 * day - 1, now)).toBe(true);
	expect(isPastWait(undefined, now)).toBe(false);
});
