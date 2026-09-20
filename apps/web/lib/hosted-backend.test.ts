import { afterEach, describe, expect, it } from "bun:test";
import {
	type HostedBootstrap,
	createHostedBackend,
	createHostedRequest,
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
			await createHostedRequest({ kind: "f", slug: "contact" })();
			expect(requestedUrl).toBe("https://api.example.test/frontend/f/contact");
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
	it("uses root frontend routes and preserves the session and variant on every request", async () => {
		const requests: { url: URL; init?: RequestInit }[] = [];
		globalThis.fetch = (async (
			input: string | URL | Request,
			init?: RequestInit,
		) => {
			requests.push({ url: new URL(String(input)), init });
			return new Response("{}", { status: 200 });
		}) as typeof fetch;
		const request = createHostedRequest(
			{ kind: "u", slug: "contact", variant: "preview one" },
			"visitor-token",
		);
		await request();
		await request("/prerun", { method: "POST", body: "{}" });
		await request("/invoke", { method: "POST", body: "{}" });
		await request("/widgets/widget-one?version=1_2_3");
		expect(requests.map(({ url }) => url.pathname)).toEqual([
			"/frontend/u/contact",
			"/frontend/u/contact/prerun",
			"/frontend/u/contact/invoke",
			"/frontend/u/contact/widgets/widget-one",
		]);
		const sessionIds = new Set<string>();
		for (const { url, init } of requests) {
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
		expect(requests[3].url.searchParams.get("version")).toBe("1_2_3");
	});
});
