import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	apiGet: vi.fn(),
	apiPost: vi.fn(),
	apiPatch: vi.fn(),
	apiPut: vi.fn(),
	apiDelete: vi.fn(),
}));
vi.mock("./api-utils", () => mocks);

import { WebInboxState } from "./inbox-state";

const auth = { user: { access_token: "token" } };
const inbox = () => new WebInboxState({ auth } as never);

describe("a bot's chat history from the hub", () => {
	beforeEach(() => vi.resetAllMocks());

	test("reads entries by position and keeps the paging cursors", async () => {
		mocks.apiGet.mockResolvedValue({
			entries: [{ message_id: "m1", seq: 1 }],
			next_before: null,
			next_to_seq: 0,
		});
		const page = await inbox().listEntries("conv 1", {
			to_seq: 500,
			limit: 50,
		});
		expect(mocks.apiGet).toHaveBeenCalledWith(
			"inbox/conversations/conv%201/entries?limit=50&to_seq=500",
			auth,
		);
		expect(page.entries[0].seq).toBe(1);
		expect(page.next_to_seq).toBe(0);
		expect(page.next_from_seq).toBeNull();
	});

	test("a jump asks around a position", async () => {
		mocks.apiGet.mockResolvedValue({ entries: [], next_before: null });
		await inbox().listEntries("c", {
			around_seq: 100,
			before_count: 40,
			after_count: 10,
		});
		expect(mocks.apiGet).toHaveBeenCalledWith(
			"inbox/conversations/c/entries?around_seq=100&before_count=40&after_count=10",
			auth,
		);
	});

	test("lists the bot's summaries", async () => {
		mocks.apiGet.mockResolvedValue({
			segments: [{ id: "s0-1-10", level: 0 }],
			next_after: null,
		});
		const list = await inbox().listSegments("c", { level: 0, limit: 100 });
		expect(mocks.apiGet).toHaveBeenCalledWith(
			"inbox/conversations/c/segments?level=0&limit=100",
			auth,
		);
		expect(list.segments).toHaveLength(1);
	});

	test("sends a topic marker as an entry", async () => {
		mocks.apiPost.mockResolvedValue({ message_id: "t1", kind: "topic" });
		await inbox().sendEntry("c", {
			message_id: "t1",
			kind: "topic",
			envelope: { v: 1, enc: "none", body: {} },
		});
		expect(mocks.apiPost.mock.calls[0][0]).toBe(
			"inbox/conversations/c/entries",
		);
		expect(mocks.apiPost.mock.calls[0][1]).toMatchObject({ kind: "topic" });
	});

	test("imports the old chat into the direct conversation as the owner and the bot", async () => {
		mocks.apiGet.mockResolvedValue({
			id: "c",
			members: [
				{ principal_id: "user-1", kind: "user" },
				{ principal_id: "bot_1", kind: "bot" },
			],
		});
		mocks.apiPost.mockResolvedValue({ imported: 2, skipped: 0 });
		const envelope = {
			v: 1 as const,
			enc: "none" as const,
			body: { text: "x" },
		};
		const result = await inbox().importHistory("bot_1", "c", [
			{
				message_id: "a1",
				sender: "owner",
				envelope,
				created_at: "2026-10-01T10:00:00Z",
			},
			{
				message_id: "a2",
				sender: "bot",
				envelope,
				created_at: "2026-10-01T10:01:00Z",
			},
		]);
		expect(result).toEqual({ imported: 2, skipped: 0 });
		const [path, body] = mocks.apiPost.mock.calls[0];
		expect(path).toBe("inbox/import");
		expect(body.local_id).toBe("botchat-bot_1");
		expect(body.members).toEqual(["bot_1"]);
		expect(
			body.entries.map((entry: { sender_id: string }) => entry.sender_id),
		).toEqual(["user-1", "bot_1"]);
	});
});
