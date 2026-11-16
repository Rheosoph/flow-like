import { afterEach, describe, expect, setSystemTime, test } from "bun:test";
import type { EditorAssetResolver } from "../asset-resolver-context";
import { createChatAssetResolver } from "./chat-asset-resolver";

afterEach(() => setSystemTime());

function harness(host?: EditorAssetResolver) {
	const signed: [string, string[]][] = [];
	const storage = {
		downloadStorageItems: async (appId: string, paths: string[]) => {
			signed.push([appId, paths]);
			return [{ prefix: paths[0], url: `asset://localhost/${paths[0]}` }];
		},
		downloadStorageItemsUser: async () => [],
	};
	const resolve = createChatAssetResolver({
		host,
		storage: storage as never,
		appId: "chat-app",
		scope: "app",
	});
	return { resolve, signed };
}

describe("createChatAssetResolver", () => {
	test("https and inline images load as written", async () => {
		const { resolve, signed } = harness();
		expect(await resolve(" https://cdn.test/a.png ")).toBe(
			"https://cdn.test/a.png",
		);
		expect(await resolve("data:image/png;base64,AAAA")).toBe(
			"data:image/png;base64,AAAA",
		);
		expect(signed).toEqual([]);
	});

	test("a stored file's storage link is signed against its own app", async () => {
		const { resolve, signed } = harness();
		expect(await resolve("storage://apps/ws1/app/bot/images/ix3.png")).toBe(
			"asset://localhost/bot/images/ix3.png",
		);
		expect(signed).toEqual([["ws1", ["bot/images/ix3.png"]]]);
	});

	test("the chat's own files go through the host resolver", async () => {
		const asked: string[] = [];
		const { resolve, signed } = harness({
			accepts: (url) => url.startsWith("bot/"),
			resolve: async (url) => {
				asked.push(url);
				return `https://signed.test/${url}`;
			},
		});
		expect(await resolve("bot/images/i3.png")).toBe(
			"https://signed.test/bot/images/i3.png",
		);
		expect(asked).toEqual(["bot/images/i3.png"]);
		expect(signed).toEqual([]);
	});

	test("other schemes and bare paths are refused", async () => {
		const { resolve, signed } = harness();
		for (const source of [
			"javascript:alert(1)",
			"http://plain.test/a.png",
			"file:///etc/passwd",
			"images/a.png",
		]) {
			await expect(resolve(source)).rejects.toThrow("cannot load");
		}
		expect(signed).toEqual([]);
	});

	test("a link is reused until it is due again, so a dead image cannot spin", async () => {
		const { resolve, signed } = harness();
		const source = "storage://apps/ws1/app/bot/missing.png";
		setSystemTime(new Date("2026-11-16T10:00:00Z"));
		await resolve(source);
		await resolve(source);
		expect(signed).toHaveLength(1);
		setSystemTime(new Date("2026-11-16T10:20:00Z"));
		await resolve(source);
		expect(signed).toHaveLength(1);
		setSystemTime(new Date("2026-11-16T10:26:00Z"));
		await resolve(source);
		expect(signed).toHaveLength(2);
	});
});
