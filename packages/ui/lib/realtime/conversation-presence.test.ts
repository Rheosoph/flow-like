import { describe, expect, test } from "bun:test";
import type { IBotTurn } from "../../state/backend-state/bot-state";
import type { IConversationTurn } from "../../state/backend-state/inbox-state";
import {
	createTypingTracker,
	hasActiveTurn,
	mergeBotActivity,
} from "./conversation-presence";
import { CHAT_TYPING_FIELD, TYPING_TTL_MS } from "./presence-signals";

const NOW = Date.parse("2026-11-04T10:00:00Z");

function apiTurn(
	id: string,
	bot: string,
	status: IConversationTurn["status"],
	created = "2026-11-04T09:59:00Z",
): IConversationTurn {
	return {
		id,
		bot_id: bot,
		status,
		trigger_message_id: "m1",
		created_at: created,
		updated_at: created,
	};
}

function liveTurn(
	id: string,
	bot: string,
	status: IBotTurn["status"],
	started_at: string | null = null,
): IBotTurn {
	return { id, bot_id: bot, conversation_id: "c1", status, started_at };
}

describe("mergeBotActivity", () => {
	const bots = new Set(["b1", "b2"]);

	test("live status wins over the API for the same turn id", () => {
		const result = mergeBotActivity(
			[apiTurn("t1", "b1", "queued")],
			[liveTurn("t1", "b1", "running", "2026-11-04T09:59:30Z")],
			bots,
			NOW,
		);
		expect(result).toEqual([
			{
				botId: "b1",
				status: "running",
				since: Date.parse("2026-11-04T09:59:30Z"),
			},
		]);
	});

	test("a finished live turn removes the API's stale entry", () => {
		const result = mergeBotActivity(
			[apiTurn("t1", "b1", "running")],
			[liveTurn("t1", "b1", "completed" as IBotTurn["status"])],
			bots,
			NOW,
		);
		expect(result).toEqual([]);
	});

	test("collapses to the most advanced status per bot", () => {
		const result = mergeBotActivity(
			[
				apiTurn("t1", "b1", "queued"),
				apiTurn("t2", "b1", "waiting"),
				apiTurn("t3", "b1", "running", "2026-11-04T09:58:00Z"),
				apiTurn("t4", "b2", "queued"),
			],
			[],
			bots,
			NOW,
		);
		expect(result.map((r) => [r.botId, r.status])).toEqual([
			["b1", "running"],
			["b2", "queued"],
		]);
		expect(result[0].since).toBe(Date.parse("2026-11-04T09:58:00Z"));
	});

	test("waiting outranks queued", () => {
		const result = mergeBotActivity(
			[apiTurn("t1", "b1", "waiting"), apiTurn("t2", "b1", "queued")],
			[],
			bots,
			NOW,
		);
		expect(result[0].status).toBe("waiting");
	});

	test("drops bots that are not members and unknown statuses", () => {
		const result = mergeBotActivity(
			[
				apiTurn("t1", "gone", "running"),
				apiTurn("t2", "b1", "bogus" as IConversationTurn["status"]),
			],
			[],
			bots,
			NOW,
		);
		expect(result).toEqual([]);
	});

	test("falls back to now when no timestamp parses", () => {
		const result = mergeBotActivity(
			[apiTurn("t1", "b1", "queued", "not a date")],
			[],
			bots,
			NOW,
		);
		expect(result[0].since).toBe(NOW);
	});
});

describe("hasActiveTurn", () => {
	test("is true only for unfinished statuses", () => {
		expect(hasActiveTurn([])).toBe(false);
		expect(hasActiveTurn([apiTurn("t", "b1", "waiting")])).toBe(true);
	});
});

describe("createTypingTracker", () => {
	const allowed = new Set(["anna", "ben"]);

	function states(entries: [number, unknown][]): ReadonlyMap<number, unknown> {
		return new Map(entries);
	}

	function typer(sub: unknown, ts: unknown) {
		return { sub, [CHAT_TYPING_FIELD]: { ts } };
	}

	test("lists active typers oldest first and skips self", () => {
		const tracker = createTypingTracker();
		tracker.observe(states([[2, typer("ben", 1000)]]), 1, allowed, 1000);
		const snap = tracker.observe(
			states([
				[1, typer("anna", 1)],
				[2, typer("ben", 1000)],
				[3, typer("anna", 2000)],
			]),
			1,
			allowed,
			2000,
		);
		expect(snap.typing).toEqual(["ben", "anna"]);
	});

	test("ignores malformed and non-member entries", () => {
		const tracker = createTypingTracker();
		const snap = tracker.observe(
			states([
				[2, null],
				[3, "str"],
				[4, typer(42, 5)],
				[5, typer("", 5)],
				[6, typer("x".repeat(500), 5)],
				[7, typer("mallory", 5)],
				[8, typer("anna", "now")],
				[9, typer("anna", Number.NaN)],
				[10, { sub: "anna", [CHAT_TYPING_FIELD]: "yes" }],
				[11, { sub: "anna" }],
			]),
			1,
			allowed,
			1000,
		);
		expect(snap).toEqual({ typing: [], nextExpiryAt: null });
	});

	test("expires without another awareness change", () => {
		const tracker = createTypingTracker();
		const s = states([[2, typer("anna", 500)]]);
		const first = tracker.observe(s, 1, allowed, 1000);
		expect(first.typing).toEqual(["anna"]);
		expect(first.nextExpiryAt).toBe(1000 + TYPING_TTL_MS);
		const later = tracker.observe(s, 1, allowed, 1000 + TYPING_TTL_MS);
		expect(later).toEqual({ typing: [], nextExpiryAt: null });
	});

	test("a heartbeat keeps the typer and the original start order", () => {
		const tracker = createTypingTracker();
		tracker.observe(states([[2, typer("anna", 1)]]), 1, allowed, 1000);
		tracker.observe(
			states([
				[2, typer("anna", 1)],
				[3, typer("ben", 1)],
			]),
			1,
			allowed,
			1500,
		);
		const snap = tracker.observe(
			states([
				[2, typer("anna", 2)],
				[3, typer("ben", 1)],
			]),
			1,
			allowed,
			2500,
		);
		expect(snap.typing).toEqual(["anna", "ben"]);
	});

	test("typing again after expiry restarts the order", () => {
		const tracker = createTypingTracker();
		tracker.observe(
			states([
				[2, typer("anna", 1)],
				[3, typer("ben", 1)],
			]),
			1,
			allowed,
			1000,
		);
		tracker.observe(
			states([
				[2, typer("anna", 1)],
				[3, typer("ben", 1)],
			]),
			1,
			allowed,
			9000,
		);
		const snap = tracker.observe(
			states([
				[2, typer("anna", 9)],
				[3, typer("ben", 1)],
			]),
			1,
			allowed,
			9500,
		);
		expect(snap.typing).toEqual(["anna"]);
	});

	test("a cleared field and a vanished peer stop typing", () => {
		const tracker = createTypingTracker();
		tracker.observe(states([[2, typer("anna", 1)]]), 1, allowed, 1000);
		expect(
			tracker.observe(states([[2, { sub: "anna" }]]), 1, allowed, 1100).typing,
		).toEqual([]);
		tracker.observe(states([[2, typer("anna", 2)]]), 1, allowed, 1200);
		expect(tracker.observe(states([]), 1, allowed, 1300).typing).toEqual([]);
	});

	test("one person on two clients counts once", () => {
		const tracker = createTypingTracker();
		const snap = tracker.observe(
			states([
				[2, typer("anna", 1)],
				[3, typer("anna", 1)],
			]),
			1,
			allowed,
			1000,
		);
		expect(snap.typing).toEqual(["anna"]);
	});
});
