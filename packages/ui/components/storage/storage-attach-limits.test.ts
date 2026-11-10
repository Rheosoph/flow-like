import { describe, expect, test, vi } from "vitest";
import { pageSnapshotFileName } from "../../lib/page-snapshot";
import {
	STORAGE_ATTACH_MAX_BYTES,
	checkStoragePicks,
	fetchStorageFiles,
	mimeTypeForName,
} from "./storage-attach-limits";

const pick = (name: string, size = 1) => ({ path: name, name, size });

describe("storage attach limits", () => {
	test("accepts up to 10 files within 35 MiB", () => {
		const ten = Array.from({ length: 10 }, (_, i) => pick(`f${i}`));
		expect(checkStoragePicks(ten)).toBeNull();
		expect(checkStoragePicks([...ten, pick("extra")])).toBe("tooMany");
		expect(checkStoragePicks([pick("big", STORAGE_ATTACH_MAX_BYTES + 1)])).toBe(
			"tooLarge",
		);
	});

	test("derives the MIME type from the extension when the blob has none", () => {
		expect(mimeTypeForName("a.PDF")).toBe("application/pdf");
		expect(mimeTypeForName("a.png", "image/webp")).toBe("image/webp");
		expect(mimeTypeForName("a.unknown")).toBe("application/octet-stream");
	});

	test("fetches files from the signed storage URL keeping name and type", async () => {
		const fetchMock = vi.fn(async () => new Response("hello"));
		vi.stubGlobal("fetch", fetchMock);
		const storage = {
			downloadStorageItems: async (_app: string, paths: string[]) => [
				{ prefix: paths[0], url: "https://signed.example/a" },
			],
		};
		const [file] = await fetchStorageFiles(storage as never, "app", [
			pick("notes.txt"),
		]);
		expect(file.name).toBe("notes.txt");
		expect(file.type).toBe("text/plain");
		expect(await file.text()).toBe("hello");
		vi.unstubAllGlobals();
	});
});

describe("page snapshot name", () => {
	test("formats page-YYYY-MM-DD-HHMMSS.png", () => {
		expect(pageSnapshotFileName(new Date(2026, 10, 9, 8, 7, 6))).toBe(
			"page-2026-11-09-080706.png",
		);
	});
});
