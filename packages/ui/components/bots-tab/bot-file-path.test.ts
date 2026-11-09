import { describe, expect, test } from "bun:test";
import { botFilePath, isBotFileReference } from "./bot-file-path";

describe("bot workspace file references", () => {
	test("reads generated images and encoded file names from workspace-relative URLs", () => {
		for (const prefix of ["", "./", "storage://"]) {
			expect(botFilePath(`${prefix}bot/images/img-1.png`)).toBe(
				"bot/images/img-1.png",
			);
		}
		expect(botFilePath("bot/files/Quarterly%20review.pdf")).toBe(
			"bot/files/Quarterly%20review.pdf",
		);
		expect(botFilePath("bot/files/report%231%3F.pdf")).toBe(
			"bot/files/report%231%3F.pdf",
		);
		expect(botFilePath("bot/files/encoded%2520name.pdf")).toBe(
			"bot/files/encoded%2520name.pdf",
		);
	});

	test("leaves fully owned references, public URLs and app routes to their renderers", () => {
		for (const url of [
			"storage://apps/workspace/app/bot/images/img-1.png",
			"https://example.com/bot/image.png",
			"/bot/image.png",
			"/settings/storage",
			"data:image/png;base64,aGVsbG8=",
		]) {
			expect(isBotFileReference(url)).toBe(false);
		}
	});

	test("refuses traversal and malformed paths before requesting storage", () => {
		for (const path of [
			"bot/../secret.png",
			"bot/%2e%2e/secret.png",
			"bot/images%2f..%2fsecret.png",
			"bot/images\\secret.png",
			"bot/images/%00.png",
			"bot/images/%zz.png",
			"bot//image.png",
			"bot/images/",
			"bot/image.png?token=1",
		]) {
			expect(() => botFilePath(path)).toThrow();
		}
	});
});
