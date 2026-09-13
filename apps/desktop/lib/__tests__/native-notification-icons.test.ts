// @vitest-environment happy-dom

import { afterEach, describe, expect, test, vi } from "vitest";
import type { NativeIconFetch } from "../native-app-icons";
import { createNativeNotificationIconResolver } from "../native-notification-icons";

const png = btoa("\x89PNG\r\n\x1a\n" + "test pixels");
const imageFetch = () =>
	vi.fn<NativeIconFetch>().mockImplementation(
		async () =>
			new Response(new Uint8Array([1]), {
				headers: { "content-type": "image/png" },
			}),
	);
const resolvers: ReturnType<typeof createNativeNotificationIconResolver>[] = [];
function resolver(
	options: Parameters<typeof createNativeNotificationIconResolver>[0] = {},
) {
	const instance = createNativeNotificationIconResolver({
		fetch: imageFetch(),
		rasterize: async () => png,
		...options,
	});
	resolvers.push(instance);
	return instance;
}
afterEach(() => {
	for (const instance of resolvers.splice(0)) instance.dispose();
	vi.useRealTimers();
});

describe("native notification artwork", () => {
	test("sends portable pixels without signed source URLs or credentials", async () => {
		const fetch = imageFetch();
		const icons = await resolver({ fetch }).resolve("account-a", [
			{
				id: "notice",
				icon: "https://assets.test/custom?signature=secret",
				appIcon: "https://assets.test/app",
			},
		]);
		expect(icons).toEqual({ notice: { png, template: false } });
		expect(fetch).toHaveBeenCalledOnce();
		expect(fetch.mock.calls[0][1]).toMatchObject({
			credentials: "omit",
			referrerPolicy: "no-referrer",
		});
		expect(JSON.stringify(icons)).not.toContain("secret");
		expect(JSON.stringify(icons)).not.toContain("https:");
	});

	test("rasterizes an installed Lucide icon as a native template and keeps emoji as text", async () => {
		const fetch = imageFetch();
		const rasterize = vi.fn(async (blob: Blob) => {
			expect(blob.type).toBe("image/svg+xml");
			const svg = new DOMParser().parseFromString(
				await blob.text(),
				"image/svg+xml",
			);
			expect(svg.querySelector("parsererror")).toBeNull();
			expect(svg.documentElement.getAttribute("viewBox")).toBe("0 0 24 24");
			expect(svg.querySelector("path,rect")).not.toBeNull();
			return png;
		});
		const icons = await resolver({ fetch, rasterize }).resolve("account-a", [
			{ id: "symbol", icon: "mail" },
			{ id: "emoji", icon: "👩🏽‍💻" },
		]);
		expect(icons).toEqual({
			symbol: { png, template: true },
			emoji: { text: "👩🏽‍💻" },
		});
		expect(rasterize).toHaveBeenCalledOnce();
		expect(fetch).not.toHaveBeenCalled();
	});

	test("falls through broken custom and app images to brand artwork", async () => {
		const fetch = imageFetch();
		fetch.mockImplementation(async (url) =>
			String(url).endsWith("app-logo.webp")
				? new Response(new Uint8Array([1]), {
						headers: { "content-type": "image/png" },
					})
				: new Response("missing", { status: 404 }),
		);
		const icons = await resolver({ fetch }).resolve("account-a", [
			{
				id: "notice",
				icon: "https://assets.test/custom",
				appIcon: "https://assets.test/app",
			},
		]);
		expect(icons.notice).toEqual({ png, template: false });
		expect(
			fetch.mock.calls.map(([url]) => new URL(String(url)).pathname),
		).toEqual(["/custom", "/app", "/app-logo.webp"]);
	});

	test("uses app artwork for legacy defaults and rejects oversized custom pixels", async () => {
		const fetch = imageFetch();
		const rasterize = vi
			.fn()
			.mockResolvedValueOnce(
				btoa(["\x89PNG\r\n\x1a\n", "x".repeat(32_768)].join("")),
			)
			.mockResolvedValue(png);
		const instance = resolver({ fetch, rasterize });
		expect(
			(
				await instance.resolve("account-a", [
					{
						id: "large",
						icon: "https://assets.test/large",
						appIcon: "https://assets.test/app",
					},
				])
			).large,
		).toEqual({ png, template: false });
		expect(
			(
				await instance.resolve("account-a", [
					{
						id: "legacy",
						icon: "/app-logo.webp",
						appIcon: "https://assets.test/app",
					},
				])
			).legacy,
		).toEqual({ png, template: false });
		expect(
			fetch.mock.calls.map(([url]) => new URL(String(url)).pathname),
		).toEqual(["/large", "/app"]);
	});

	test("leaves no image when every candidate fails so Swift can use its bundled mark", async () => {
		const fetch = vi
			.fn<NativeIconFetch>()
			.mockRejectedValue(new Error("offline"));
		expect(
			await resolver({ fetch }).resolve("account-a", [
				{ id: "notice", icon: "https://assets.test/missing" },
			]),
		).toEqual({});
	});

	test("reuses pixels only within the current account scope", async () => {
		const fetch = imageFetch();
		const instance = resolver({ fetch });
		const sources = [{ id: "notice", icon: "https://assets.test/shared-url" }];
		await instance.resolve("account-a", sources);
		await instance.resolve("account-a", sources);
		expect(fetch).toHaveBeenCalledOnce();
		await instance.resolve("account-b", sources);
		expect(fetch).toHaveBeenCalledTimes(2);
	});

	test("shares a single in-flight image request across notification rows", async () => {
		const fetch = imageFetch();
		const icons = await resolver({ fetch }).resolve(
			"account-a",
			Array.from({ length: 8 }, (_, index) => ({
				id: String(index),
				icon: "https://assets.test/shared-url",
			})),
		);
		expect(Object.keys(icons)).toHaveLength(8);
		expect(fetch).toHaveBeenCalledOnce();
	});

	test("does not retain large data URLs in the session cache", async () => {
		const fetch = imageFetch();
		const instance = resolver({ fetch });
		const sources = [
			{ id: "notice", icon: `data:image/png;base64,${"A".repeat(8_000)}` },
		];
		await instance.resolve("account-a", sources);
		await instance.resolve("account-a", sources);
		expect(fetch).toHaveBeenCalledTimes(2);
	});

	test("times out an unresponsive Lucide import and uses the source app icon", async () => {
		vi.useFakeTimers();
		const fetch = imageFetch();
		const icons = resolver({
			fetch,
			lucide: () => new Promise(() => {}),
		}).resolve("account-a", [
			{ id: "notice", icon: "mail", appIcon: "https://assets.test/app" },
		]);
		await vi.advanceTimersByTimeAsync(10_000);
		expect(await icons).toEqual({ notice: { png, template: false } });
		expect(fetch).toHaveBeenCalledOnce();
	});

	test("rejects late pixels after a new account resolves or the resolver is disposed", async () => {
		let finish!: (value: string) => void;
		const rasterize = vi
			.fn()
			.mockImplementationOnce(
				() =>
					new Promise<string>((resolve) => {
						finish = resolve;
					}),
			)
			.mockResolvedValue(png);
		const instance = resolver({ rasterize });
		const sources = [{ id: "notice", icon: "https://assets.test/shared-url" }];
		const old = instance.resolve("account-a", sources);
		const rejected = expect(old).rejects.toMatchObject({ name: "AbortError" });
		await vi.waitFor(() => expect(rasterize).toHaveBeenCalledOnce());
		await instance.resolve("account-b", sources);
		finish(png);
		await rejected;
		instance.dispose();
		await expect(instance.resolve("account-b", sources)).rejects.toMatchObject({
			name: "AbortError",
		});
	});
});
