import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import { localInbox } from "../local-bots/commands";

describe("a local bot's chat history", () => {
	beforeEach(() => mocks.invoke.mockReset());

	test("reads by position through the position command", async () => {
		mocks.invoke.mockResolvedValue({
			entries: [{ message_id: "m1", seq: 1 }],
			next_before: null,
			next_to_seq: 3,
			next_from_seq: null,
		});
		const page = await localInbox.entries("c1", { to_seq: 900, limit: 50 });
		expect(mocks.invoke).toHaveBeenCalledWith(
			"local_inbox_entries_by_position",
			{
				conversationId: "c1",
				query: {
					from_seq: undefined,
					to_seq: 900,
					around_seq: undefined,
					before_count: undefined,
					after_count: undefined,
					limit: 50,
				},
			},
		);
		expect(page.next_to_seq).toBe(3);
	});

	test("a plain read keeps the cursor command", async () => {
		mocks.invoke.mockResolvedValue({ entries: [], next_before: null });
		await localInbox.entries("c1", { limit: 20 });
		expect(mocks.invoke.mock.calls[0][0]).toBe("local_inbox_entries");
	});

	test("lists the segments of the local conversation", async () => {
		mocks.invoke.mockResolvedValue({
			segments: [{ id: "s0-1-5" }],
			next_after: null,
		});
		const list = await localInbox.segments("c1", { level: 0 });
		expect(mocks.invoke.mock.calls[0][0]).toBe("local_inbox_segments");
		expect(list.segments).toHaveLength(1);
	});

	test("imports the old chat through the import command", async () => {
		mocks.invoke.mockResolvedValue({ imported: 1, skipped: 1 });
		const envelope = {
			v: 1 as const,
			enc: "none" as const,
			body: { text: "x" },
		};
		const result = await localInbox.importHistory("c1", "bot_1", [
			{
				message_id: "a1",
				sender: "owner",
				envelope,
				created_at: "2026-10-01T10:00:00Z",
			},
		]);
		expect(result).toEqual({ imported: 1, skipped: 1 });
		expect(mocks.invoke.mock.calls[0][0]).toBe("local_inbox_import_history");
		expect(mocks.invoke.mock.calls[0][1]).toMatchObject({
			conversationId: "c1",
			botId: "bot_1",
		});
	});
});
