import { describe, expect, test } from "bun:test";
import type { IStorageState } from "../state/backend-state/storage-state";
import { isStoragePrefix, presignSinglePath } from "./presign-assets";

function storageState(
	downloadStorageItems: IStorageState["downloadStorageItems"],
): IStorageState {
	return { downloadStorageItems } as IStorageState;
}

describe("chat-compatible asset presigning", () => {
	test("recognizes app storage paths but leaves resolved URLs alone", () => {
		expect(isStoragePrefix("images/background.webp")).toBe(true);
		expect(isStoragePrefix("https://cdn.example.com/background.webp")).toBe(
			false,
		);
		expect(isStoragePrefix("asset://localhost/background.webp")).toBe(false);
		expect(isStoragePrefix("data:image/png;base64,AAAA")).toBe(false);
	});

	test("passes external URLs through without touching storage", async () => {
		let calls = 0;
		const state = storageState(async () => {
			calls += 1;
			return [];
		});

		await expect(
			presignSinglePath(
				"app-id",
				"https://cdn.example.com/background.webp",
				state,
			),
		).resolves.toBe("https://cdn.example.com/background.webp");
		expect(calls).toBe(0);
	});

	test("replaces a storage path with its signed URL", async () => {
		const state = storageState(async (appId, prefixes) => {
			expect(appId).toBe("app-id");
			expect(prefixes).toEqual(["images/background.webp"]);
			return [
				{
					prefix: prefixes[0],
					url: "https://signed.example.com/background.webp?token=abc",
				},
			];
		});

		await expect(
			presignSinglePath("app-id", "images/background.webp", state),
		).resolves.toBe("https://signed.example.com/background.webp?token=abc");
	});

	test("keeps the persisted path when signing fails", async () => {
		const state = storageState(async (_appId, prefixes) => [
			{ prefix: prefixes[0], error: "not found" },
		]);

		await expect(
			presignSinglePath("app-id", "images/missing.webp", state),
		).resolves.toBe("images/missing.webp");
	});
});
