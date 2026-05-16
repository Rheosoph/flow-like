import { describe, expect, test } from "bun:test";
import { isAzureBlobStorageUrl } from "./storage-url";

describe("isAzureBlobStorageUrl", () => {
	test("accepts Azure Blob Storage account hosts", () => {
		expect(
			isAzureBlobStorageUrl(
				"https://flowlike.blob.core.windows.net/container/file.png?sig=test",
			),
		).toBe(true);
		expect(
			isAzureBlobStorageUrl("https://FLOWLIKE.blob.core.windows.net/file.png"),
		).toBe(true);
	});

	test("rejects substring matches outside the hostname suffix", () => {
		expect(
			isAzureBlobStorageUrl(
				"https://attacker.example/upload?target=.blob.core.windows.net",
			),
		).toBe(false);
		expect(
			isAzureBlobStorageUrl(
				"https://flowlike.blob.core.windows.net.attacker.example/file.png",
			),
		).toBe(false);
		expect(
			isAzureBlobStorageUrl("https://flowlikeblob.core.windows.net/file.png"),
		).toBe(false);
	});

	test("rejects malformed URLs", () => {
		expect(isAzureBlobStorageUrl("not a url")).toBe(false);
	});
});
