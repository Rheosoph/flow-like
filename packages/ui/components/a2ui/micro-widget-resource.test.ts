import { expect, test } from "bun:test";
import {
	type ResourceReference,
	type ResourceResultPayload,
	createEnvelope,
} from "@flow-like/widget-sdk";
import type { IStorageState } from "../../state/backend-state/storage-state";
import {
	createMicroWidgetResources,
	referenceKey,
	resolveAppResource,
	resourceReferences,
} from "./micro-widget-resource";

const url = "https://storage.example/apps/a/image.png?signature=trusted";
const reference = (path = "apps/a/upload/image.png"): ResourceReference => ({
	path,
	store_ref: "store",
	cache_store_ref: null,
});
const file = (ref = reference()) => ({
	encoding: "reference",
	content: "",
	storage: ref,
	url: "asset://localhost/%2Fetc%2Fpasswd",
});
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const request = (id: string, ref = reference()) =>
	createEnvelope(
		"resource:request",
		{ requestId: id, reference: ref },
		"nonce",
		"widget",
	);
const options = (result: (value: ResourceResultPayload) => void) => ({
	desktop: false,
	enabled: () => true,
	resolve: async () => url,
	result,
});

test("references are canonical lookup keys and supplied URLs never grant network access", () => {
	const value = {
		files: { a: file(), b: { url }, c: { ...file(), storage: null } },
	};
	const grants = resourceReferences(value);
	expect(grants.size).toBe(1);
	expect(grants.get(referenceKey(reference()) ?? "")).toEqual(reference());
	expect(referenceKey({ path: "x\n", store_ref: "s" })).toBeNull();
	expect(referenceKey({ path: "x", store_ref: "s" })).toBe(
		referenceKey({ path: "x", store_ref: "s", cache_store_ref: null }),
	);
});

test("resource resolution uses the current app storage API and rejects foreign paths", async () => {
	const calls: string[][] = [];
	const storage = {
		downloadStorageItems: async (app: string, paths: string[]) => {
			calls.push([app, ...paths]);
			return paths.map((prefix) => ({ prefix, url }));
		},
	} as unknown as IStorageState;
	expect(await resolveAppResource(reference(), "a", storage)).toBe(url);
	expect(calls).toEqual([["a", "apps/a/upload/image.png"]]);
	for (const path of [
		"/etc/passwd",
		"apps/b/upload/private.png",
		"apps/a/upload/../private",
		"apps/a/upload/%2e%2e/private",
		"apps/a/upload/a%2fb",
		"custom/file",
	]) {
		await expect(
			resolveAppResource(reference(path), "a", storage),
		).rejects.toThrow();
	}
	expect(calls).toHaveLength(1);
});

test("local byte resolution validates app paths before native access", async () => {
	const calls: [string, string, boolean][] = [];
	const bytes = new Uint8Array([0, 255, 128]).buffer;
	let urlCalls = 0;
	const storage = {
		readStorageItem: async (
			appId: string,
			path: string,
			userScoped: boolean,
		) => {
			calls.push([appId, path, userScoped]);
			return bytes;
		},
		downloadStorageItems: async () => {
			urlCalls++;
			return [];
		},
		downloadStorageItemsUser: async () => {
			urlCalls++;
			return [];
		},
	} as unknown as IStorageState;
	for (const path of [
		"apps/b/upload/private",
		"apps/a/upload/%2e%2e/secret",
		"users/u/apps/b/private",
		"/etc/passwd",
	]) {
		await expect(
			resolveAppResource(reference(path), "a", storage),
		).rejects.toThrow();
	}
	expect(calls).toHaveLength(0);
	expect(await resolveAppResource(reference(), "a", storage)).toBe(bytes);
	expect(
		await resolveAppResource(
			reference("users/u/apps/a/image.png"),
			"a",
			storage,
		),
	).toBe(bytes);
	expect(calls).toEqual([
		["a", "apps/a/upload/image.png", false],
		["a", "users/u/apps/a/image.png", true],
	]);
	expect(urlCalls).toBe(0);
});

test("native denial never falls back to an asset URL; only an explicit null uses remote URLs", async () => {
	let urlCalls = 0;
	const storage = {
		readStorageItem: async (): Promise<ArrayBuffer | null> => {
			throw new Error("permission denied");
		},
		downloadStorageItems: async () => {
			urlCalls++;
			return [{ prefix: reference().path, url }];
		},
	} as unknown as IStorageState;
	await expect(resolveAppResource(reference(), "a", storage)).rejects.toThrow(
		"permission denied",
	);
	expect(urlCalls).toBe(0);
	storage.readStorageItem = async () => null;
	expect(await resolveAppResource(reference(), "a", storage)).toBe(url);
	expect(urlCalls).toBe(1);
	const legacy = {
		downloadStorageItems: async () => [
			{ prefix: reference().path, url: "asset://localhost/secret" },
		],
	} as unknown as IStorageState;
	await expect(resolveAppResource(reference(), "a", legacy)).rejects.toThrow(
		"Unsupported",
	);
});

test("desktop resource requests never fetch local asset URLs", async () => {
	for (const localUrl of [
		"asset://localhost/file",
		"http://asset.localhost/file",
		"https://asset.localhost/file",
	]) {
		const results: ResourceResultPayload[] = [];
		let fetched = false;
		const host = createMicroWidgetResources({
			...options((value) => results.push(value)),
			desktop: true,
			resolve: async () => localUrl,
			fetch: (async () => {
				fetched = true;
				return new Response("secret");
			}) as typeof fetch,
		});
		host.setProps(file());
		host.handle(request("asset"));
		await tick();
		expect(fetched).toBe(false);
		expect(results[0]).toMatchObject({
			ok: false,
			error: "Unsupported storage resource URL.",
		});
		host.dispose();
	}
});

test("direct bytes preserve grants, transfer ownership, and enforce the resource limit without fetch", async () => {
	const results: ResourceResultPayload[] = [];
	const transfers: Transferable[][] = [];
	let enabled = true;
	let nativeCalls = 0;
	let fetchCalls = 0;
	let bytes = new Uint8Array([0, 255, 128]).buffer;
	const host = createMicroWidgetResources({
		...options(() => {}),
		desktop: true,
		enabled: () => enabled,
		resolve: async () => {
			nativeCalls++;
			return bytes;
		},
		fetch: (async () => {
			fetchCalls++;
			return new Response("unsafe");
		}) as typeof fetch,
		maxBytes: 3,
		result: (value, transfer = []) => {
			results.push(value);
			transfers.push(transfer);
		},
	});
	host.setProps(file());
	host.handle(request("foreign", reference("apps/a/upload/foreign")));
	enabled = false;
	host.handle(request("disabled"));
	expect(nativeCalls).toBe(0);
	enabled = true;
	host.handle(request("bytes"));
	await tick();
	expect(results[2]).toMatchObject({
		requestId: "bytes",
		ok: true,
		mimeType: "application/octet-stream",
	});
	expect(results[2].data).toBe(bytes);
	expect(transfers[2]).toEqual([bytes]);
	bytes = new Uint8Array(4).buffer;
	host.handle(request("oversize"));
	await tick();
	expect(results[3].ok).toBe(false);
	expect(results[3].error).toContain("limit");
	expect(fetchCalls).toBe(0);
	host.dispose();
});

test("revocation, cancellation, and reset discard delayed native bytes", async () => {
	const results: ResourceResultPayload[] = [];
	const finish: ((bytes: ArrayBuffer) => void)[] = [];
	const host = createMicroWidgetResources({
		...options((value) => results.push(value)),
		resolve: () => new Promise<ArrayBuffer>((resolve) => finish.push(resolve)),
	});
	host.setProps(file());
	host.handle(request("revoked"));
	host.setProps({});
	finish[0](new ArrayBuffer(1));
	await tick();
	expect(results).toHaveLength(1);
	expect(results[0]).toMatchObject({ requestId: "revoked", ok: false });
	host.setProps(file());
	host.handle(request("cancelled"));
	host.handle(
		createEnvelope(
			"resource:cancel",
			{ requestId: "cancelled" },
			"nonce",
			"widget",
		),
	);
	finish[1](new ArrayBuffer(1));
	await tick();
	host.handle(request("reset"));
	host.reset();
	finish[2](new ArrayBuffer(1));
	await tick();
	expect(results).toHaveLength(1);
	host.dispose();
});

test("unapproved references and disabled access never resolve or fetch", () => {
	const results: ResourceResultPayload[] = [];
	let calls = 0;
	let enabled = true;
	const host = createMicroWidgetResources({
		...options((value) => results.push(value)),
		enabled: () => enabled,
		resolve: async () => {
			calls++;
			return url;
		},
	});
	host.setProps(file());
	host.handle(request("wrong", reference("apps/a/upload/other.png")));
	enabled = false;
	host.handle(request("disabled"));
	expect(calls).toBe(0);
	expect(results.map((value) => value.ok)).toEqual([false, false]);
	host.dispose();
});

test("fetch uses a trusted fresh URL, omits credentials, and transfers bytes", async () => {
	const results: ResourceResultPayload[] = [];
	const transfers: Transferable[][] = [];
	let fetched: unknown;
	let fetchOptions: RequestInit | undefined;
	const host = createMicroWidgetResources({
		...options((value) => results.push(value)),
		result: (value, transfer = []) => {
			results.push(value);
			transfers.push(transfer);
		},
		fetch: (async (input, init) => {
			fetched = input;
			fetchOptions = init;
			return new Response(new Uint8Array([0, 255, 128]), {
				headers: { "content-type": "image/png" },
			});
		}) as typeof fetch,
	});
	host.setProps(file());
	host.handle(request("approved"));
	await tick();
	expect(fetched).toBe(url);
	expect(fetchOptions).toMatchObject({
		credentials: "omit",
		redirect: "error",
		mode: "cors",
	});
	expect(results[0]).toMatchObject({
		requestId: "approved",
		ok: true,
		mimeType: "image/png",
	});
	expect(new Uint8Array(results[0].data ?? new ArrayBuffer(0))).toEqual(
		new Uint8Array([0, 255, 128]),
	);
	expect(transfers[0]).toEqual([results[0].data]);
	host.dispose();
});

test("query grants remain transactional until the replacement build is accepted", async () => {
	const results: ResourceResultPayload[] = [];
	let calls = 0;
	const host = createMicroWidgetResources({
		...options((value) => results.push(value)),
		fetch: (async () => {
			calls++;
			return new Response("image");
		}) as typeof fetch,
	});
	const next = reference("apps/a/upload/new.png");
	host.beginQuery("q1", "applyBuild", {
		artifact: { files: { image: file() } },
	});
	host.completeQuery("q1", true);
	host.beginQuery("q2", "applyBuild", {
		artifact: { files: { image: file(next) } },
	});
	host.handle(request("old-during-pending"));
	host.handle(request("pending", next));
	await tick();
	host.completeQuery("q2", false);
	host.handle(request("old-after-rejection"));
	host.handle(request("rejected", next));
	await tick();
	expect(calls).toBe(3);
	expect(results.find((value) => value.requestId === "rejected")?.ok).toBe(
		false,
	);
	host.beginQuery("q3", "applyBuild", file(next));
	host.completeQuery("q3", true);
	host.handle(request("old-after-commit"));
	host.handle(request("committed", next));
	await tick();
	expect(calls).toBe(4);
	expect(
		results.find((value) => value.requestId === "old-after-commit")?.ok,
	).toBe(false);
	host.dispose();
});

test("resource requests queue with only two fetches in flight", async () => {
	const finish: (() => void)[] = [];
	let active = 0;
	let peak = 0;
	const results: ResourceResultPayload[] = [];
	const host = createMicroWidgetResources({
		...options((value) => results.push(value)),
		fetch: (() =>
			new Promise<Response>((resolve) => {
				active++;
				peak = Math.max(peak, active);
				finish.push(() => {
					active--;
					resolve(new Response("image"));
				});
			})) as typeof fetch,
	});
	host.setProps(file());
	for (let i = 0; i < 5; i++) host.handle(request(String(i)));
	await tick();
	expect(finish.length).toBe(2);
	for (let i = 0; i < 5; i++) {
		finish[i]();
		await tick();
	}
	expect(peak).toBe(2);
	expect(results).toHaveLength(5);
	host.dispose();
});

test("streaming enforces the per-file limit before buffering the whole response", async () => {
	let cancelled = false;
	const results: ResourceResultPayload[] = [];
	const host = createMicroWidgetResources({
		...options((value) => results.push(value)),
		maxBytes: 6,
		fetch: (async () =>
			new Response(
				new ReadableStream({
					pull(controller) {
						controller.enqueue(new Uint8Array(4));
					},
					cancel() {
						cancelled = true;
					},
				}),
			)) as typeof fetch,
	});
	host.setProps(file());
	host.handle(request("large"));
	await tick();
	expect(cancelled).toBe(true);
	expect(results[0].ok).toBe(false);
	expect(results[0].error).toContain("limit");
	host.dispose();
});

test("prop revocation and frame reset abort work without sending stale bytes", async () => {
	const results: ResourceResultPayload[] = [];
	const signals: AbortSignal[] = [];
	const finish: ((response: Response) => void)[] = [];
	const host = createMicroWidgetResources({
		...options((value) => results.push(value)),
		fetch: ((_url, init) => {
			const signal = init?.signal;
			if (!signal) throw new Error("Abort signal missing");
			signals.push(signal);
			return new Promise<Response>((resolve) => finish.push(resolve));
		}) as typeof fetch,
	});
	host.setProps(file());
	host.handle(request("revoked"));
	await tick();
	host.setProps({});
	expect(signals[0].aborted).toBe(true);
	expect(results[0]).toMatchObject({ requestId: "revoked", ok: false });
	finish[0](new Response("old"));
	await tick();
	host.setProps(file());
	host.handle(request("reset"));
	await tick();
	host.reset();
	expect(signals[1].aborted).toBe(true);
	finish[1](new Response("stale"));
	await tick();
	expect(results).toHaveLength(1);
	host.handle(request("reset-no-grant"));
	expect(results[1].ok).toBe(false);
	host.dispose();
});
