import { describe, expect, test } from "bun:test";
import {
	attachmentParts,
	conversationFileId,
	conversationFileTooLarge,
	inBatches,
	isConversationFileHandle,
} from "./conversation-files";

const ready = (n: number) => ({
	file_id: `f${n}`,
	name: `Übersicht (${n})#1.pdf`,
	app_id: "conversation_c1",
	path: `f${n}.pdf`,
	bytes: 10 * n,
	sha256: "a".repeat(64),
	mime: "application/pdf",
});

describe("conversation files", () => {
	test("attachment parts continue the numbering of existing attachments", () => {
		const first = attachmentParts([ready(1)]);
		expect(first.map((part) => part.id)).toEqual(["a1"]);
		const more = attachmentParts([ready(2), ready(3)], first);
		expect(more.map((part) => part.id)).toEqual(["a2", "a3"]);
		expect(more[0]?.handle).toEqual({
			app_id: "conversation_c1",
			path: "f2.pdf",
			bytes: 20,
			sha256: "a".repeat(64),
			mime: "application/pdf",
		});
		expect(more[0]?.name).toBe("Übersicht (2)#1.pdf");
	});

	test("a file id is the path without its extension", () => {
		expect(conversationFileId("abc.pdf")).toBe("abc");
		expect(conversationFileId("abc")).toBe("abc");
	});

	test("only conversation handles carry the reserved app prefix", () => {
		expect(isConversationFileHandle({ app_id: "conversation_c1" })).toBe(true);
		expect(isConversationFileHandle({ app_id: "app_ws" })).toBe(false);
	});

	test("files above 35 MB are named as too large", () => {
		expect(
			conversationFileTooLarge({ name: "a.bin", size: 35 * 1024 * 1024 + 1 }),
		).toContain("a.bin");
		expect(
			conversationFileTooLarge({ name: "a.bin", size: 35 * 1024 * 1024 }),
		).toBeUndefined();
	});

	test("batches keep the order of their input and reject on the first failure", async () => {
		const done = await inBatches([3, 1, 2, 4, 5], async (n) => {
			await new Promise((resolve) => setTimeout(resolve, n));
			return n * 2;
		});
		expect(done).toEqual([6, 2, 4, 8, 10]);
		await expect(
			inBatches([1, 2], async (n) => {
				if (n === 2) throw new Error("boom");
				return n;
			}),
		).rejects.toThrow("boom");
	});
});
