import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import { describe, expect, it, vi } from "vitest";
import {
	type NativeIconFetch,
	createNativeAppIconPublisher,
	readNativeAppIcon,
} from "../native-app-icons";

const png = btoa("\x89PNG\r\n\x1a\n" + "test pixels");
const backend = (apps: [string, string | null, string?][]) =>
	({
		appState: {
			getApps: async () =>
				apps.map(([id, icon, thumbnail]) => [{ id }, { icon, thumbnail }]),
		},
	}) as unknown as IBackendState;
const imageFetch = () =>
	vi.fn<NativeIconFetch>().mockImplementation(
		async () =>
			new Response(new Uint8Array([1]), {
				headers: { "content-type": "image/png" },
			}),
	);
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("native app icon publisher", () => {
	it("uses current catalog icons only, omits credentials, and clears missing icons", async () => {
		const publish = vi.fn().mockResolvedValue(undefined);
		const fetch = imageFetch();
		const publisher = createNativeAppIconPublisher({
			publish,
			fetch,
			rasterize: async () => png,
		});
		await publisher.sync({
			scope: "alice",
			appIds: ["one", "two"],
			backend: backend([
				["one", "https://assets.example/icon?signature=private"],
				["two", null, "https://assets.example/preview"],
				["foreign", "https://assets.example/foreign"],
			]),
		});
		expect(fetch).toHaveBeenCalledTimes(1);
		expect(fetch.mock.calls[0][1]).toMatchObject({
			credentials: "omit",
			referrerPolicy: "no-referrer",
		});
		expect(publish.mock.calls[0][0].icons).toEqual(
			expect.arrayContaining([
				{ appId: "one", data: png },
				{ appId: "two", data: null },
			]),
		);
		expect(JSON.stringify(publish.mock.calls)).not.toContain("https://");
		publisher.dispose();
	});

	it("reuses same-scope bytes and refetches after an account change", async () => {
		const fetch = imageFetch();
		const publish = vi.fn().mockResolvedValue(undefined);
		const publisher = createNativeAppIconPublisher({
			publish,
			fetch,
			rasterize: async () => png,
		});
		const input = {
			scope: "alice",
			appIds: ["one"],
			backend: backend([["one", "https://assets.example/icon"]]),
		};
		await publisher.sync(input);
		await publisher.sync(input);
		expect(fetch).toHaveBeenCalledTimes(1);
		expect(publish).toHaveBeenCalledTimes(1);
		await publisher.sync({ ...input, scope: "bob" });
		expect(fetch).toHaveBeenCalledTimes(2);
		expect(publish.mock.calls[1][0].scope).toBe("bob");
		publisher.dispose();
	});

	it("does not publish a late image after cleanup or a superseding account", async () => {
		let resolve: ((value: string) => void) | undefined;
		const publish = vi.fn().mockResolvedValue(undefined);
		const rasterize = vi
			.fn()
			.mockImplementationOnce(
				() =>
					new Promise<string>((done) => {
						resolve = done;
					}),
			)
			.mockResolvedValue(png);
		const publisher = createNativeAppIconPublisher({
			publish,
			fetch: imageFetch(),
			rasterize,
		});
		const input = {
			scope: "alice",
			appIds: ["one"],
			backend: backend([["one", "https://assets.example/icon"]]),
		};
		const old = publisher.sync(input);
		await tick();
		await publisher.sync({ ...input, scope: "bob" });
		resolve?.(png);
		await old;
		expect(publish.mock.calls.map(([value]) => value.scope)).toEqual(["bob"]);
		publisher.dispose();
		await publisher.sync(input);
		expect(publish).toHaveBeenCalledTimes(1);
	});

	it("clears changed artwork on failure and retries it on the next refresh", async () => {
		const publish = vi.fn().mockResolvedValue(undefined);
		const rasterize = vi
			.fn()
			.mockResolvedValueOnce(png)
			.mockRejectedValueOnce(new Error("bad image"))
			.mockResolvedValue(png);
		const publisher = createNativeAppIconPublisher({
			publish,
			fetch: imageFetch(),
			rasterize,
		});
		await publisher.sync({
			scope: "alice",
			appIds: ["one"],
			backend: backend([["one", "https://assets.example/old"]]),
		});
		const changed = {
			scope: "alice",
			appIds: ["one"],
			backend: backend([["one", "https://assets.example/new"]]),
		};
		await publisher.sync(changed);
		expect(publish.mock.calls[1][0].icons).toEqual([
			{ appId: "one", data: null },
		]);
		await publisher.sync(changed);
		expect(publish.mock.calls[2][0].icons).toEqual([
			{ appId: "one", data: png },
		]);
		publisher.dispose();
	});

	it("bounds native batches and the app catalog", async () => {
		const publish = vi.fn().mockResolvedValue(undefined);
		const large = btoa(["\x89PNG\r\n\x1a\n", "x".repeat(32_760)].join(""));
		const apps: [string, string][] = Array.from({ length: 140 }, (_, index) => [
			`app-${index}`,
			`https://assets.example/${index}`,
		]);
		const publisher = createNativeAppIconPublisher({
			publish,
			fetch: imageFetch(),
			rasterize: async () => large,
		});
		await publisher.sync({
			scope: "alice",
			appIds: apps.map(([id]) => id),
			backend: backend(apps),
		});
		expect(publish.mock.calls.flatMap(([value]) => value.icons)).toHaveLength(
			128,
		);
		for (const [value] of publish.mock.calls)
			expect(
				new TextEncoder().encode(JSON.stringify(value)).byteLength,
			).toBeLessThanOrEqual(262_144);
		publisher.dispose();
	});
});

describe("native app icon source bounds", () => {
	it("stops an oversized stream even when its declared length is absent", async () => {
		const cancel = vi.fn();
		const response = new Response(
			new ReadableStream({
				start(controller) {
					controller.enqueue(new Uint8Array(4_194_305));
				},
				cancel,
			}),
			{ headers: { "content-type": "image/png" } },
		);
		await expect(
			readNativeAppIcon(
				"https://assets.example/icon",
				vi.fn().mockResolvedValue(response),
				new AbortController().signal,
			),
		).rejects.toThrow("too large");
		expect(cancel).toHaveBeenCalledTimes(1);
	});

	it("rejects non-image content and URL credentials before decoding", async () => {
		const fetch = vi
			.fn()
			.mockResolvedValue(
				new Response("private", { headers: { "content-type": "text/html" } }),
			);
		await expect(
			readNativeAppIcon(
				"https://assets.example/icon",
				fetch,
				new AbortController().signal,
			),
		).rejects.toThrow("not an image");
		await expect(
			readNativeAppIcon(
				"https://user:password@assets.example/icon",
				fetch,
				new AbortController().signal,
			),
		).rejects.toThrow("Unsupported");
		expect(fetch).toHaveBeenCalledTimes(1);
	});
});
