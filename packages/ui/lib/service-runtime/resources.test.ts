import { afterEach, describe, expect, test } from "bun:test";
import { RUNTIME_ASSET_LIMIT, createRuntimeResources } from "./resources";

const cleanups: Array<() => void> = [];
afterEach(() => {
	for (const close of cleanups.splice(0)) close();
});
const issued = "/ui/assets/0123456789abcdef0123456789abcdef";
function fixture() {
	const calls: Array<{ path: string; init?: RequestInit }> = [];
	const resources = createRuntimeResources({
		appId: "app",
		fetch: async (path, init) => {
			calls.push({ path, init });
			return new Response("image-bytes", {
				headers: { "content-type": "image/png" },
			});
		},
	});
	cleanups.push(resources.close);
	return { resources, calls };
}

describe("deployed resources", () => {
	test("keeps replicated asset owner identities in the authorized path and cache key", async () => {
		const { resources, calls } = fixture();
		const a = `/ui/assets/${"a".repeat(32)}/${"c".repeat(32)}`;
		const b = `/ui/assets/${"b".repeat(32)}/${"c".repeat(32)}`;
		expect(await resources.resolve(a)).not.toBe(await resources.resolve(b));
		expect(calls.map((call) => call.path)).toEqual([a, b]);
		await expect(resources.resolve(`${a}/extra`)).rejects.toThrow();
		await expect(
			resources.resolve(`/ui/assets/${"x".repeat(32)}/${"c".repeat(32)}`),
		).rejects.toThrow();
	});
	test("maps only issued resources and caches their protected bytes as session blob URLs", async () => {
		const { resources, calls } = fixture();
		const original = {
			nested: [{ src: issued, ordinary: "images/logo.png" }],
			remote: "https://example.test/logo.png",
		};
		const mapped = await resources.mapValue(original);
		expect(mapped.nested[0].src.startsWith("blob:")).toBe(true);
		expect(mapped.nested[0].ordinary).toBe("images/logo.png");
		expect(mapped.remote).toBe(original.remote);
		expect(original.nested[0].src).toBe(issued);
		expect(await resources.resolve(issued)).toBe(mapped.nested[0].src);
		expect(calls).toHaveLength(1);
		expect(await (await fetch(mapped.nested[0].src)).text()).toBe(
			"image-bytes",
		);
		resources.close();
		await expect(fetch(mapped.nested[0].src)).rejects.toThrow();
		await expect(resources.resolve(issued)).rejects.toThrow();
	});

	test("uses upload roots and never sends external URL or local filesystem destinations", async () => {
		const { resources, calls } = fixture();
		await resources.resolve("apps/app/upload/images/logo.png");
		expect(calls[0].path).toBe(
			"/ui/assets?store=upload&path=images%2Flogo.png",
		);
		for (const path of [
			"https://external.test/image.png",
			"data:image/png;base64,AA==",
			"blob:existing",
		])
			expect(await resources.resolve(path)).toBe(path);
		for (const path of [
			"apps/other/upload/a.png",
			"file:///private/data",
			"asset://localhost/private/data",
			"http://asset.localhost/private/data",
			"../secret",
			"images/%2e%2e/secret",
			"images/%252e%252e/secret",
			"images%2fsecret",
			"/absolute",
			"C:\\secret",
			"/ui/assets?store=storage&path=secret",
			"/ui/assets?store=upload&path=../secret",
			"/ui/assets?store=upload&store=upload&path=x",
			"/ui/assets?url=file:///secret",
		])
			await expect(resources.resolve(path)).rejects.toThrow();
		expect(calls).toHaveLength(1);
		await expect(
			resources.mapValue({ src: "asset://localhost/private/data" }),
		).rejects.toThrow(/service-issued/);
	});

	test("bounds incoming bytes regardless of Content-Length and cancels oversized resources", async () => {
		let cancelled = false;
		let sent = 0;
		const resources = createRuntimeResources({
			fetch: async () =>
				new Response(
					new ReadableStream(
						{
							pull(controller) {
								controller.enqueue(new Uint8Array(1024 * 1024));
								sent++;
							},
							cancel() {
								cancelled = true;
							},
						},
						{ highWaterMark: 0 },
					),
				),
		});
		cleanups.push(resources.close);
		await expect(resources.resolve(issued)).rejects.toThrow(/memory limit/);
		expect(sent).toBe(33);
		expect(cancelled).toBe(true);
		const declared = createRuntimeResources({
			fetch: async () =>
				new Response("x", {
					headers: { "content-length": String(RUNTIME_ASSET_LIMIT + 1) },
				}),
		});
		cleanups.push(declared.close);
		await expect(declared.resolve(issued)).rejects.toThrow(/32 MiB/);
	});

	test("limits concurrent asset downloads and cancels queued work on close", async () => {
		let started = 0;
		const resources = createRuntimeResources({
			fetch: (_, init) => {
				started++;
				return new Promise<Response>((_, reject) =>
					init?.signal?.addEventListener(
						"abort",
						() => reject(new Error("closed")),
						{ once: true },
					),
				);
			},
		});
		cleanups.push(resources.close);
		const pending = Array.from({ length: 12 }, (_, index) =>
			resources.resolve(`images/${index}.png`),
		);
		for (let index = 0; index < 5; index++) await Promise.resolve();
		expect(started).toBe(4);
		resources.close();
		const results = await Promise.allSettled(pending);
		expect(results.every((result) => result.status === "rejected")).toBe(true);
		expect(started).toBe(4);
	});

	test("caller cancellation stops waiting without invalidating another consumer's cached asset", async () => {
		let finish!: (response: Response) => void;
		const resources = createRuntimeResources({
			fetch: () =>
				new Promise<Response>((resolve) => {
					finish = resolve;
				}),
		});
		cleanups.push(resources.close);
		const controller = new AbortController();
		const a = resources.resolve(issued, controller.signal);
		const b = resources.resolve(issued);
		await Promise.resolve();
		controller.abort();
		await expect(a).rejects.toThrow(/cancelled/);
		finish(new Response("asset"));
		expect((await b).startsWith("blob:")).toBe(true);
	});
});
