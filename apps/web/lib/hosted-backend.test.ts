import { afterEach, describe, expect, it, spyOn } from "bun:test";
import { isWidgetAccessRefusedError } from "@flow-like/flow-like-ui/components/a2ui/micro-widget-policy";
import {
	HUB_REFRESH_TIMEOUT_MS,
	RequestTimeoutError,
	WRITE_REQUEST_TIMEOUT_MS,
} from "@flow-like/flow-like-ui/lib/request-deadline";
import {
	type HostedBootstrap,
	HostedHttpError,
	createHostedBackend,
	createHostedRequest,
	hostedErrorCode,
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
	describe("package widgets", () => {
		const ACCESS = "eyJhbGciOiJFUzI1NiJ9.eyJwa2ciOiJ4In0.YWNjZXNz";
		const DIGEST = `sha256:${"a".repeat(64)}`;
		const widget = {
			packageId: "com.example.maps",
			packageVersion: "1.2.0",
			widgetId: "map",
			preview: false,
			appId: "published-app",
		};
		const hosting = (auth_proxy: boolean) =>
			({
				app_id: "published-app",
				auth_proxy,
				bootstrap: { event: { id: "published-event" } },
			}) as HostedBootstrap;
		const unusedRequest = async () => new Response("{}");

		function recordRegistry() {
			const requests: { path: string; init?: RequestInit }[] = [];
			globalThis.fetch = (async (
				input: string | URL | Request,
				init?: RequestInit,
			) => {
				const path = new URL(String(input)).pathname;
				requests.push({ path, init });
				if (path.endsWith("/widget-access"))
					return Response.json({ access: ACCESS, expiresIn: 43_200 });
				if (path.endsWith("/widget-grant"))
					return Response.json({
						grant: null,
						expiresIn: 86_400,
						policyDigest: DIGEST,
					});
				return Response.json({
					source: "registry:api.flow-like.com",
					packageId: widget.packageId,
					packageVersion: widget.packageVersion,
					bundleHash: "b".repeat(64),
					widgetId: widget.widgetId,
					preview: false,
					status: "ok",
					policy: {},
					policyDigest: DIGEST,
					networkInputs: [],
				});
			}) as typeof fetch;
			return requests;
		}

		async function openWidget(backend: ReturnType<typeof createHostedBackend>) {
			await backend.registryState.describeWidgetPolicy?.(widget);
			await backend.registryState.mintWidgetGrant?.({
				...widget,
				policyDigest: DIGEST,
			});
			return backend.registryState.getWidgetAccess?.({
				packageId: widget.packageId,
				packageVersion: widget.packageVersion,
				appId: widget.appId,
			});
		}

		const authorizations = (requests: { init?: RequestInit }[]) =>
			requests.map(({ init }) =>
				new Headers(init?.headers).get("Authorization"),
			);

		it("a sign-in interface opens them as its viewer, with the token current at each call", async () => {
			const requests = recordRegistry();
			let token: string | undefined = "viewer-token-1";
			const backend = createHostedBackend(
				hosting(true),
				unusedRequest,
				() => token,
			);

			await expect(openWidget(backend)).resolves.toEqual({
				access: ACCESS,
				expiresIn: 43_200,
			});
			expect(requests.map(({ path }) => path)).toEqual([
				"/api/v1/registry/package/com.example.maps/widget-policy/1.2.0/map",
				"/api/v1/registry/package/com.example.maps/widget-grant",
				"/api/v1/registry/package/com.example.maps/widget-access",
			]);
			expect(authorizations(requests)).toEqual([
				"Bearer viewer-token-1",
				"Bearer viewer-token-1",
				"Bearer viewer-token-1",
			]);
			for (const { init } of requests) {
				expect(init?.credentials).toBe("omit");
				expect(JSON.parse(String(init?.body)).appId).toBe("published-app");
			}

			token = "viewer-token-2";
			await openWidget(backend);
			expect(authorizations(requests.slice(3))).toEqual([
				"Bearer viewer-token-2",
				"Bearer viewer-token-2",
				"Bearer viewer-token-2",
			]);

			token = undefined;
			await openWidget(backend);
			expect(authorizations(requests.slice(6))).toEqual([null, null, null]);
		});

		it("an anonymous interface never borrows the viewer's login", async () => {
			const requests = recordRegistry();
			await openWidget(
				createHostedBackend(
					hosting(false),
					unusedRequest,
					() => "viewer-token",
				),
			);
			await openWidget(createHostedBackend(hosting(true), unusedRequest));
			expect(requests).toHaveLength(6);
			expect(authorizations(requests)).toEqual([
				null,
				null,
				null,
				null,
				null,
				null,
			]);
			for (const { init } of requests) expect(init?.credentials).toBe("omit");
		});

		describe("when the API answers with an error", () => {
			const answer = (response: () => unknown) => {
				globalThis.fetch = (async () => response()) as unknown as typeof fetch;
			};
			const apiError = (status: number, code: string) => () =>
				Response.json({ error: { code, message: "refused" } }, { status });
			const registry = () =>
				createHostedBackend(hosting(true), unusedRequest, () => "viewer-token")
					.registryState;
			const access = (state = registry()) =>
				state.getWidgetAccess?.({
					packageId: widget.packageId,
					packageVersion: widget.packageVersion,
					appId: widget.appId,
				});

			it("only an API without the access route loads the sandbox anonymously", async () => {
				answer(() => new Response(null, { status: 404 }));
				await expect(access()).resolves.toEqual({
					access: null,
					expiresIn: 3600,
				});
				answer(() => new Response("Not Found", { status: 404 }));
				await expect(access()).resolves.toEqual({
					access: null,
					expiresIn: 3600,
				});
				answer(() => new Response(null, { status: 405 }));
				await expect(access()).resolves.toEqual({
					access: null,
					expiresIn: 3600,
				});
			});

			it("an API that answered an access request has the route, so a later bare 404 or 405 fails instead of loading anonymously", async () => {
				const state = registry();
				answer(() => Response.json({ ok: true }));
				await expect(access(state)).rejects.toThrow("malformed access token");
				answer(() => new Response(null, { status: 404 }));
				await expect(access(state)).resolves.toMatchObject({ access: null });

				answer(() => Response.json({ access: ACCESS, expiresIn: 43_200 }));
				await expect(access(state)).resolves.toMatchObject({ access: ACCESS });
				for (const status of [404, 405]) {
					answer(() => new Response(null, { status }));
					const failure = await access(state)?.catch((error: unknown) => error);
					expect(failure).toMatchObject({ status, code: undefined });
					expect(isWidgetAccessRefusedError(failure)).toBe(false);
				}
				answer(() => new Response(null, { status: 404 }));
				await expect(access()).resolves.toMatchObject({ access: null });
			});

			it("a refusal keeps its status and code instead of passing as anonymous access", async () => {
				answer(apiError(404, "NOT_FOUND"));
				await expect(access()).rejects.toMatchObject({
					status: 404,
					code: "NOT_FOUND",
					message: "The widget is not available for public hosting.",
				});
				answer(apiError(403, "FORBIDDEN"));
				await expect(access()).rejects.toMatchObject({
					status: 403,
					code: "FORBIDDEN",
				});
			});

			it("a failure the API did not explain carries its status and no code", async () => {
				answer(() => new Response("<html>Bad Gateway</html>", { status: 502 }));
				const failure = await access()?.then(
					() => undefined,
					(error: unknown) => error,
				);
				expect(failure).toBeInstanceOf(HostedHttpError);
				expect(failure).toMatchObject({ status: 502, code: undefined });

				answer(() => ({
					ok: false,
					status: 503,
					text: async () => {
						throw new TypeError("Load failed");
					},
				}));
				await expect(access()).rejects.toMatchObject({
					status: 503,
					code: undefined,
				});
			});

			it("describe and mint errors carry the code too", async () => {
				answer(apiError(400, "INVALID_RUNTIME_SOURCES"));
				await expect(
					registry().describeWidgetPolicy?.(widget),
				).rejects.toMatchObject({
					status: 400,
					code: "INVALID_RUNTIME_SOURCES",
				});
				await expect(
					registry().mintWidgetGrant?.({ ...widget, policyDigest: DIGEST }),
				).rejects.toMatchObject({
					status: 400,
					code: "INVALID_RUNTIME_SOURCES",
				});
			});
		});

		describe("when the API never answers", () => {
			/** Requests hang and deadlines are collected instead of awaited; `expire` is what the timer would run. */
			function hang() {
				const deadlines: { ms: number; expire: () => void }[] = [];
				const timers = spyOn(globalThis, "setTimeout").mockImplementation(((
					expire: () => void,
					ms: number,
				) => {
					deadlines.push({ ms, expire });
					return 0;
				}) as unknown as typeof setTimeout);
				const signals: (AbortSignal | null | undefined)[] = [];
				globalThis.fetch = ((_input: unknown, init?: RequestInit) => {
					signals.push(init?.signal);
					return new Promise(() => {});
				}) as unknown as typeof fetch;
				return { deadlines, signals, restore: () => timers.mockRestore() };
			}
			const outcomeOf = (request: Promise<unknown> | undefined) =>
				request?.then(
					() => undefined,
					(error: unknown) => error,
				);
			const registry = () =>
				createHostedBackend(hosting(true), unusedRequest).registryState;

			it("an access request fails at the hub refresh deadline without a ruling, so it is asked again", async () => {
				const { deadlines, signals, restore } = hang();
				try {
					const outcome = outcomeOf(
						registry().getWidgetAccess?.({
							packageId: widget.packageId,
							packageVersion: widget.packageVersion,
							appId: widget.appId,
						}),
					);
					expect(deadlines.map(({ ms }) => ms)).toEqual([
						HUB_REFRESH_TIMEOUT_MS,
					]);
					expect(signals.map((signal) => signal?.aborted)).toEqual([false]);

					deadlines[0].expire();
					const failure = await outcome;
					expect(failure).toBeInstanceOf(RequestTimeoutError);
					expect(failure).not.toHaveProperty("status");
					expect(isWidgetAccessRefusedError(failure)).toBe(false);
					expect(signals[0]?.aborted).toBe(true);
				} finally {
					restore();
				}
			});

			it("describe and mint fail at the API's write deadline", async () => {
				const { deadlines, restore } = hang();
				try {
					const outcomes = [
						outcomeOf(registry().describeWidgetPolicy?.(widget)),
						outcomeOf(
							registry().mintWidgetGrant?.({ ...widget, policyDigest: DIGEST }),
						),
					];
					expect(deadlines.map(({ ms }) => ms)).toEqual([
						WRITE_REQUEST_TIMEOUT_MS,
						WRITE_REQUEST_TIMEOUT_MS,
					]);
					for (const { expire } of deadlines) expire();
					for (const outcome of outcomes) {
						expect(await outcome).toBeInstanceOf(RequestTimeoutError);
					}
				} finally {
					restore();
				}
			});
		});
	});
	it("reads the API error code and nothing else", () => {
		expect(
			hostedErrorCode('{"error":{"code":"NOT_FOUND","message":"Not Found"}}'),
		).toBe("NOT_FOUND");
		expect(hostedErrorCode('{"code":"FORBIDDEN"}')).toBe("FORBIDDEN");
		for (const body of [
			"",
			"null",
			"Not Found",
			"<html>404</html>",
			'{"error":"Not Found"}',
			'{"error":{"code":" "}}',
			'{"error":{"code":404}}',
		]) {
			expect(hostedErrorCode(body)).toBeUndefined();
		}
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
