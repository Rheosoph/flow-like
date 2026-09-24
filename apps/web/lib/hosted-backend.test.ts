import { afterEach, describe, expect, it } from "bun:test";
import {
	type HostedBootstrap,
	createHostedBackend,
	createHostedRequest,
	hostedErrorMessage,
} from "./hosted-backend";

const originalFetch = globalThis.fetch;
afterEach(() => {
	globalThis.fetch = originalFetch;
});

describe("hosted API contract", () => {
	it("uses the web container's public runtime configuration", async () => {
		const previousFlag = process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG;
		const previousWindow = Object.getOwnPropertyDescriptor(
			globalThis,
			"window",
		);
		try {
			process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG = "1";
			Object.defineProperty(globalThis, "window", {
				configurable: true,
				value: {
					location: { origin: "https://app.example.test" },
					__FLOW_LIKE_PUBLIC_CONFIG__: {
						version: 1,
						apiUrl: "https://api.example.test",
					},
				},
			});
			let requestedUrl = "";
			globalThis.fetch = (async (input) => {
				requestedUrl = String(input);
				return new Response("{}");
			}) as typeof fetch;
			await createHostedRequest({ app: "published-app", route: "/contact" })();
			expect(requestedUrl).toBe(
				"https://api.example.test/frontend/a/published-app?route=%2Fcontact",
			);
		} finally {
			if (previousFlag === undefined)
				Reflect.deleteProperty(
					process.env,
					"NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG",
				);
			else process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG = previousFlag;
			if (previousWindow)
				Object.defineProperty(globalThis, "window", previousWindow);
			else Reflect.deleteProperty(globalThis, "window");
		}
	});
	it("does not expose main-app operations or another event through the hosted backend", async () => {
		const data = {
			app_id: "published-app",
			auth_proxy: false,
			bootstrap: { event: { id: "published-event" } },
		} as HostedBootstrap;
		let requests = 0;
		const backend = createHostedBackend(data, async () => {
			requests++;
			return new Response("{}");
		});
		await expect(
			backend.eventState.getEvent("other-app", "published-event"),
		).rejects.toThrow("only run its published event");
		await expect(
			backend.eventState.getEvent("published-app", "other-event"),
		).rejects.toThrow("only run its published event");
		expect(() => backend.appState.getApp("published-app")).toThrow();
		expect(() =>
			backend.boardState.getBoard("published-app", "board"),
		).toThrow();
		await expect(
			backend.pageState.getPage("published-app", "other-page"),
		).rejects.toThrow("unavailable in a hosted interface");
		expect(requests).toBe(0);
	});
	it("addresses the app, and preserves the route, session and variant on every request", async () => {
		const requests: { url: URL; init?: RequestInit }[] = [];
		globalThis.fetch = (async (
			input: string | URL | Request,
			init?: RequestInit,
		) => {
			requests.push({ url: new URL(String(input)), init });
			return new Response("{}", { status: 200 });
		}) as typeof fetch;
		const request = createHostedRequest(
			{ app: "app-1", route: "/support/demo", variant: "preview one" },
			"visitor-token",
		);
		await request();
		await request("/prerun", { method: "POST", body: "{}" });
		await request("/invoke", { method: "POST", body: "{}" });
		await request("/routes");
		await request("/assets", { method: "POST", body: "{}" });
		await request("/widgets/widget-one?version=1_2_3");
		expect(requests.map(({ url }) => url.pathname)).toEqual([
			"/frontend/a/app-1",
			"/frontend/a/app-1/prerun",
			"/frontend/a/app-1/invoke",
			"/frontend/a/app-1/routes",
			"/frontend/a/app-1/assets",
			"/frontend/a/app-1/widgets/widget-one",
		]);
		const sessionIds = new Set<string>();
		for (const { url, init } of requests) {
			expect(url.searchParams.getAll("route")).toEqual(["/support/demo"]);
			expect(url.searchParams.get("__variant")).toBe("preview one");
			const headers = new Headers(init?.headers);
			expect(headers.get("Authorization")).toBe("Bearer visitor-token");
			expect(headers.get("X-Flow-Like-Session")).toMatch(
				/^[a-zA-Z0-9_-]{16,128}$/,
			);
			sessionIds.add(headers.get("X-Flow-Like-Session") ?? "");
			expect(init?.credentials).toBe("omit");
		}
		expect(sessionIds.size).toBe(1);
		expect(requests[5].url.searchParams.get("version")).toBe("1_2_3");
	});
	it("sends the root route explicitly and encodes the app id", async () => {
		let requestedUrl: URL | undefined;
		globalThis.fetch = (async (input: string | URL | Request) => {
			requestedUrl = new URL(String(input));
			return new Response("{}");
		}) as typeof fetch;
		await createHostedRequest({ app: "app_1", route: "/" })();
		expect(requestedUrl?.pathname).toBe("/frontend/a/app_1");
		expect(requestedUrl?.searchParams.get("route")).toBe("/");
		expect(requestedUrl?.searchParams.has("__variant")).toBe(false);
	});
	it("signs storage assets through the hosted route in server-sized batches", async () => {
		const data = {
			app_id: "published-app",
			auth_proxy: false,
			bootstrap: { event: { id: "published-event" } },
		} as HostedBootstrap;
		const bodies: string[][] = [];
		const backend = createHostedBackend(data, async (suffix, init) => {
			expect(suffix).toBe("/assets");
			expect(init?.method).toBe("POST");
			const { prefixes } = JSON.parse(String(init?.body)) as {
				prefixes: string[];
			};
			bodies.push(prefixes);
			return new Response(
				JSON.stringify(
					prefixes.map((prefix) => ({ prefix, url: `https://s/${prefix}` })),
				),
			);
		});
		const paths = Array.from({ length: 150 }, (_, index) => `a/${index}.png`);
		const results = await backend.storageState.downloadStorageItems(
			"published-app",
			paths,
		);
		expect(bodies.map((batch) => batch.length)).toEqual([100, 50]);
		expect(results).toHaveLength(150);
		expect(results[0]).toEqual({ prefix: "a/0.png", url: "https://s/a/0.png" });
		await expect(
			backend.storageState.downloadStorageItems("other-app", ["a/0.png"]),
		).rejects.toThrow("another app");
	});
	it("cancelling a hosted run closes its invoke stream instead of throwing", async () => {
		const data = {
			app_id: "published-app",
			auth_proxy: false,
			bootstrap: { event: { id: "published-event" } },
		} as HostedBootstrap;
		let signal: AbortSignal | undefined;
		const encoder = new TextEncoder();
		const backend = createHostedBackend(data, async (_suffix, init) => {
			signal = init?.signal ?? undefined;
			return new Response(
				new ReadableStream<Uint8Array>({
					start(controller) {
						controller.enqueue(
							encoder.encode(
								'data: {"event_type":"run_initiated","payload":{"run_id":"run-1"}}\n\n',
							),
						);
						signal?.addEventListener("abort", () =>
							controller.error(
								new DOMException("The operation was aborted.", "AbortError"),
							),
						);
					},
				}),
			);
		});
		const runIds: string[] = [];
		const running = backend.eventState
			.executeEvent(
				"published-app",
				"published-event",
				{ id: "node", payload: {} },
				false,
				(runId) => runIds.push(runId),
			)
			.then(
				() => undefined,
				(error: unknown) => error,
			);
		while (runIds.length === 0) await Bun.sleep(1);

		await backend.eventState.cancelExecution("unknown-run");
		expect(signal?.aborted).toBe(false);
		await backend.eventState.cancelExecution("run-1");
		expect(signal?.aborted).toBe(true);
		expect(await running).toBeInstanceOf(DOMException);
		await expect(
			backend.eventState.cancelExecution("run-1"),
		).resolves.toBeUndefined();
	});
	it("reads the API error envelope instead of printing a bare status", () => {
		expect(
			hostedErrorMessage(
				401,
				'{"error":{"code":"UNAUTHORIZED","message":"Sign in to open this frontend"}}',
			),
		).toBe("Sign in to open this frontend");
		expect(hostedErrorMessage(400, '{"message":"Bad input"}')).toBe(
			"Bad input",
		);
		expect(hostedErrorMessage(500, '{"error":{"code":"X"}}')).toBe(
			"Request failed (500)",
		);
		expect(hostedErrorMessage(502, "")).toBe("Request failed (502)");
		expect(
			hostedErrorMessage(
				404,
				'{"error":{"code":"NOT_FOUND","message":"Not Found"}}',
			),
		).toContain("not published");
	});
});
