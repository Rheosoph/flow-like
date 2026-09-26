import {
	appQueryContext,
	appRouteUrl,
	parseAppRouteTarget,
} from "@flow-like/flow-like-ui/lib/app-route-url";
import type { IEvent } from "@flow-like/flow-like-ui/lib/schema/flow/event";
import {
	pathUseUrl,
	readUseRoutePath,
} from "@flow-like/flow-like-ui/lib/use-route-url";
import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import { describe, expect, test, vi } from "vitest";
import {
	type NativeDispatchContext,
	dispatchNativeAction,
	executeNativeMcpOperation,
	loadNativeSnapshot,
	nativeEventUrl,
	nativeNavigationRequest,
	nativeQuickActionPayload,
	nativeWebOrigin,
	withNativeActivePage,
	withNativeActiveRuns,
} from "../native-integration";
const bytes = (value: unknown) =>
	Array.from(new TextEncoder().encode(JSON.stringify(value)));
const event = (overrides: Partial<IEvent> = {}): IEvent =>
	({
		id: "event",
		name: "Translate",
		active: true,
		event_type: "quick_action",
		board_id: "board",
		node_id: "node",
		inputs: [],
		config: bytes({
			secret: "DO NOT COPY",
			native_integration: {
				enabled: true,
				favorite: true,
				surfaces: ["siri", "widget"],
			},
		}),
		...overrides,
	}) as IEvent;
function fixture(events = [event()]) {
	return {
		appState: {
			getApps: vi.fn(async () => [
				[{ id: "app" }, { name: "Translator" }],
				[{ id: "other" }, { name: "Other account app" }],
			]),
			getApp: vi.fn(async () => ({ id: "app" })),
		},
		userState: {
			getProfile: vi.fn(async () => ({ apps: [{ app_id: "app" }] })),
			listNotifications: vi.fn(async () => [
				{ id: "notice", title: "Approval", description: "Review it" },
			]),
		},
		eventState: {
			getEvents: vi.fn(async () => events),
			getEventAuthoritative: vi.fn(async () => events[0]),
		},
		routeState: {
			getRouteByPathAuthoritative: vi.fn(
				async (_appId: string, path: string) => ({
					path,
					eventId: events[0]?.id,
				}),
			),
		},
		usageState: {
			getExecutionHistory: vi.fn(async () => ({
				items: [
					{
						id: "run",
						app_id: "app",
						status: "Info",
						created_at: "2026-09-01T00:00:00Z",
					},
				],
			})),
			getExecutionActivity: vi.fn(async () => ({
				total: 400,
				attention: [{ id: "older-error", app_id: "app", status: "Error" }],
			})),
			getUsageSummary: vi.fn(async () => ({
				total_executions: 700,
				total_llm_invocations: 6,
				total_llm_price: 2,
				total_embedding_price: 1,
			})),
		},
	} as unknown as IBackendState;
}
function context(backend: IBackendState): NativeDispatchContext {
	return {
		scope: "account-a",
		backend,
		navigate: vi.fn(),
		execute: vi.fn(async () => {}),
		flowpilot: vi.fn(),
		isCurrent: () => true,
	};
}
describe("native app paths", () => {
	test("opens the exported use page with an app route and structured query values", async () => {
		const backend = fixture([event({ route: "/orders/123" })]);
		const ctx = context(backend);
		await dispatchNativeAction(
			{
				id: "route",
				scope: ctx.scope,
				action: {
					kind: "open_app",
					appId: "app",
					path: "/orders/123?tag=first&id=wrong&eventId=wrong",
					queryParams: [{ name: "tag", value: "A&B + 50% / 東京 #1" }],
				},
			},
			ctx,
		);
		const url = new URL(
			vi.mocked(ctx.navigate).mock.calls[0][0],
			"https://app.test",
		);
		expect(url.pathname).toBe("/use");
		expect(url.searchParams.get("id")).toBe("app");
		expect(url.searchParams.get("eventId")).toBeNull();
		expect(url.searchParams.get("route")).toBe("/orders/123");
		expect(appQueryContext(url.search)).toMatchObject({
			_query_params: {
				id: "wrong",
				eventId: "wrong",
				tag: "A&B + 50% / 東京 #1",
			},
			_query_param_values: { tag: ["first", "A&B + 50% / 東京 #1"] },
		});
		expect(backend.routeState.getRouteByPathAuthoritative).toHaveBeenCalledWith(
			"app",
			"/orders/123",
		);
		expect(ctx.execute).not.toHaveBeenCalled();
	});

	test("default app opening needs no route and rechecks current profile membership", async () => {
		const backend = fixture();
		const ctx = context(backend);
		await dispatchNativeAction(
			{
				id: "default",
				scope: ctx.scope,
				action: { kind: "open_app", appId: "app" },
			},
			ctx,
		);
		expect(ctx.navigate).toHaveBeenCalledWith("/use?id=app");
		expect(
			backend.routeState.getRouteByPathAuthoritative,
		).not.toHaveBeenCalled();
		vi.mocked(ctx.navigate).mockClear();
		await expect(
			dispatchNativeAction(
				{
					id: "other",
					scope: ctx.scope,
					action: { kind: "open_app", appId: "other" },
				},
				ctx,
			),
		).rejects.toThrow("workspace");
		expect(ctx.navigate).not.toHaveBeenCalled();
	});

	test("missing, redirected or inactive route targets never fall back to another page", async () => {
		for (const mapping of [
			null,
			{ path: "/", eventId: "event" },
			{ path: "/orders", eventId: "event" },
		]) {
			const backend = fixture([event({ active: false })]);
			backend.routeState.getRouteByPathAuthoritative = vi.fn(
				async () => mapping,
			);
			const ctx = context(backend);
			await expect(
				dispatchNativeAction(
					{
						id: "route",
						scope: ctx.scope,
						action: { kind: "open_app", appId: "app", path: "/orders" },
					},
					ctx,
				),
			).rejects.toThrow();
			expect(ctx.navigate).not.toHaveBeenCalled();
		}
	});

	test("an account change during authoritative route validation prevents navigation", async () => {
		const backend = fixture();
		const ctx = context(backend);
		backend.routeState.getRouteByPathAuthoritative = vi.fn(async () => {
			ctx.isCurrent = () => false;
			return { path: "/orders", eventId: "event" };
		});
		await dispatchNativeAction(
			{
				id: "route",
				scope: ctx.scope,
				action: { kind: "open_app", appId: "app", path: "/orders" },
			},
			ctx,
		);
		expect(ctx.navigate).not.toHaveBeenCalled();
		expect(backend.eventState.getEventAuthoritative).not.toHaveBeenCalled();
	});

	test("native route stores can resolve canonical saved paths and synthesized defaults", async () => {
		for (const [configured, path] of [
			[event({ route: "orders/" }), "/orders"],
			[event({ is_default: true }), "/"],
		] as const) {
			const backend = fixture([configured]);
			backend.routeState.getRouteByPathAuthoritative = vi.fn(async () => null);
			const ctx = context(backend);
			await dispatchNativeAction(
				{
					id: "canonical",
					scope: ctx.scope,
					action: { kind: "open_app", appId: "app", path },
				},
				ctx,
			);
			expect(backend.eventState.getEventAuthoritative).toHaveBeenCalledWith(
				"app",
				"event",
			);
			const url = new URL(
				vi.mocked(ctx.navigate).mock.calls[0][0],
				"https://app.test",
			);
			expect(url.pathname).toBe("/use");
			expect(url.searchParams.get("route")).toBe(path);
		}
	});

	test("a cached route candidate cannot restore an Event whose route changed", async () => {
		const backend = fixture([event({ route: "/old" })]);
		backend.routeState.getRouteByPathAuthoritative = vi.fn(async () => null);
		backend.eventState.getEventAuthoritative = vi.fn(async () =>
			event({ route: "/new" }),
		);
		const ctx = context(backend);
		await expect(
			dispatchNativeAction(
				{
					id: "stale",
					scope: ctx.scope,
					action: { kind: "open_app", appId: "app", path: "/old" },
				},
				ctx,
			),
		).rejects.toThrow("no longer");
		expect(ctx.navigate).not.toHaveBeenCalled();
	});

	test("native widget URLs round-trip path and query JSON without double encoding", () => {
		const rawPath = "/orders?tag=first&tab=recent+items";
		const pairs = [{ name: "tag", value: "A&B + 50% / 東京 #1" }];
		const params = new URLSearchParams({
			appId: "app",
			scope: "account+a",
			path: rawPath,
			queryParams: JSON.stringify(pairs),
		});
		const request = nativeNavigationRequest(
			`flow-like://native/app?${params}`,
			"account+a",
		);
		expect(request?.action).toEqual({
			kind: "open_app",
			appId: "app",
			path: "/orders",
			queryParams: [
				{ name: "tag", value: "first" },
				{ name: "tab", value: "recent items" },
				...pairs,
			],
		});
		expect(
			nativeNavigationRequest(`flow-like://native/app?${params}`, "other"),
		).toBeUndefined();
	});

	test("rejects malformed widget targets and provides scoped overview openers", () => {
		for (const tail of [
			"path=https%3A%2F%2Fevil.test",
			"path=%2F..%2Fflow",
			"queryParams=bad-json",
			"queryParams=%7B%7D",
		])
			expect(
				nativeNavigationRequest(
					`flow-like://native/app?appId=app&scope=account-a&${tail}`,
					"account-a",
				),
			).toBeUndefined();
		for (const [path, kind] of [
			["flowpilot", "flowpilot"],
			["inbox", "open_inbox"],
		]) {
			expect(
				nativeNavigationRequest(
					`flow-like://native/${path}?scope=account-a`,
					"account-a",
				)?.action.kind,
			).toBe(kind);
			expect(
				nativeNavigationRequest(
					`flow-like://native/${path}?scope=other`,
					"account-a",
				),
			).toBeUndefined();
		}
		expect(
			nativeNavigationRequest("flow-like://user@native/home", "account-a"),
		).toBeUndefined();
	});
});
describe("native snapshots", () => {
	test("keeps notification image URLs out of native snapshots while retaining source app fallback", async () => {
		const backend = fixture();
		backend.appState.getApps = vi.fn(async () => [
			[
				{ id: "app" },
				{
					name: "Translator",
					icon: "https://assets.test/app?signature=app-secret",
				},
			],
			[
				{ id: "other" },
				{ name: "Other profile", icon: "https://assets.test/other" },
			],
		]) as never;
		backend.userState.listNotifications = vi.fn(async () => [
			{
				id: "notice",
				title: "Review",
				app_id: "app",
				icon: "https://assets.test/custom?signature=notice-secret",
			},
			{ id: "foreign", title: "Other", app_id: "other" },
		]) as never;
		const sources = vi.fn();
		const snapshot = await loadNativeSnapshot(
			backend,
			"account-a",
			[],
			true,
			new Date(),
			sources,
		);
		expect(sources).toHaveBeenCalledWith([
			{
				id: "notice",
				icon: "https://assets.test/custom?signature=notice-secret",
				appIcon: "https://assets.test/app?signature=app-secret",
			},
			{ id: "foreign", icon: undefined, appIcon: undefined },
		]);
		expect(
			snapshot.sections.find((section) => section.kind === "inbox")?.items[0],
		).toMatchObject({
			id: "notice",
			action: { kind: "open_inbox", appId: "app" },
		});
		expect(JSON.stringify(snapshot)).not.toContain("assets.test");
		expect(JSON.stringify(snapshot)).not.toContain("secret");
	});

	test("converts recorded model and embedding microdollars into widget USD", async () => {
		const backend = fixture();
		backend.usageState!.getUsageSummary = vi.fn(async () => ({
			total_executions: 700,
			total_llm_invocations: 6,
			total_embedding_invocations: 2,
			total_llm_price: 1_675_000,
			total_embedding_price: 375_000,
		}));
		const snapshot = await loadNativeSnapshot(backend, "account-a", [], true);
		expect(
			snapshot.sections
				.find((section) => section.kind === "usage")
				?.items.find((item) => item.id === "cost"),
		).toMatchObject({
			value: "$2.05",
			subtitle: "USD · all recorded usage",
			action: { kind: "open_home" },
		});
	});

	test("run widgets only publish rows belonging to the selected profile apps", async () => {
		const backend = fixture();
		const rows = [
			{ id: "mine", app_id: "app", status: "Info" },
			{ id: "other-profile", app_id: "other", status: "Error" },
			{ id: "missing-app", status: "Error" },
		];
		backend.usageState!.getExecutionHistory = vi.fn(async () => ({
			items: rows,
		})) as never;
		backend.usageState!.getExecutionActivity = vi.fn(async () => ({
			total: 3,
			attention: rows,
		})) as never;
		const snapshot = await loadNativeSnapshot(backend, "account-a", [], true);
		for (const kind of ["recent_runs", "attention"])
			expect(
				snapshot.sections
					.find((section) => section.kind === kind)
					?.items.map((item) => item.id),
			).toEqual(["mine"]);
	});

	test("a failed profile read marks recent apps unavailable", async () => {
		const backend = fixture();
		backend.userState.getProfile = vi.fn(async () => {
			throw new Error("offline");
		});
		const snapshot = await loadNativeSnapshot(backend, "account-a", [], true);
		expect(
			snapshot.sections.find((section) => section.kind === "recent_apps"),
		).toMatchObject({ state: "unavailable", items: [] });
	});
	test("uses actual opening order, selected profile apps, and complete attention source", async () => {
		const backend = fixture([
			event(),
			event({ id: "disabled", active: false }),
			event({ id: "background", event_type: "cron" }),
		]);
		const snapshot = await loadNativeSnapshot(
			backend,
			"account-a",
			[
				{ appId: "app", lastOpenedAt: "2026-09-10T12:00:00Z", openCount: 4 },
				{ appId: "other", lastOpenedAt: "2026-09-11T12:00:00Z", openCount: 2 },
			],
			true,
			new Date("2026-09-12T00:00:00Z"),
		);
		expect(snapshot.apps).toEqual([
			{ id: "app", title: "Translator", spotlightEligible: false },
		]);
		expect(snapshot.events.map((item) => item.id)).toEqual(["app:event"]);
		expect(
			snapshot.sections
				.find((item) => item.kind === "recent_apps")
				?.items.map((item) => item.id),
		).toEqual(["app"]);
		expect(
			snapshot.sections.find((item) => item.kind === "attention")?.items[0].id,
		).toBe("older-error");
		expect(
			snapshot.sections.find((item) => item.kind === "workspace")?.items,
		).toMatchObject([
			{ id: "apps", value: "1" },
			{
				id: "activity",
				title: "Account executions · last 7 days",
				value: "400",
			},
		]);
		expect(backend.usageState!.getExecutionActivity).toHaveBeenCalledWith(7);
		expect(JSON.stringify(snapshot)).not.toContain("DO NOT COPY");
		expect(snapshot.expiresAt).toBe("2026-09-12T01:00:00.000Z");
	});
	test("failed reads remain unavailable and do not impersonate zero activity", async () => {
		const backend = fixture();
		backend.usageState!.getExecutionActivity = vi.fn(async () => {
			throw new Error("offline");
		});
		const snapshot = await loadNativeSnapshot(backend, "account-a", [], true);
		expect(
			snapshot.sections.find((item) => item.kind === "attention"),
		).toMatchObject({ state: "unavailable", items: [] });
		expect(
			snapshot.sections.find((item) => item.kind === "recent_runs")?.state,
		).toBe("ready");
	});
	test("signed-out snapshots retain only positively local-only app entry points", async () => {
		const backend = fixture();
		backend.isLocalOnly = vi.fn(async (appId) => appId === "app");
		const snapshot = await loadNativeSnapshot(backend, "local", [], false);
		expect(snapshot.apps.map((app) => app.id)).toEqual(["app"]);
		expect(snapshot.events.map((event) => event.appId)).toEqual(["app"]);
	});
	test("signed-out snapshots never fetch account inbox or usage", async () => {
		const backend = fixture();
		const snapshot = await loadNativeSnapshot(backend, "local", [], false);
		expect(backend.userState.listNotifications).not.toHaveBeenCalled();
		expect(backend.usageState!.getExecutionHistory).not.toHaveBeenCalled();
		expect(snapshot.apps).toEqual([]);
		expect(snapshot.events).toEqual([]);
		expect(snapshot.sections.find((item) => item.kind === "inbox")?.state).toBe(
			"signed_out",
		);
	});
	test("event catalog reuses fresh reads, evicts removed apps and serves stale events when a refetch fails", async () => {
		const backend = fixture();
		const catalog = new Map([["removed", { events: [event()], fetchedAt: 0 }]]);
		const start = new Date("2026-09-12T00:00:00Z").getTime();
		const load = (minutes: number) =>
			loadNativeSnapshot(
				backend,
				"account-a",
				[],
				true,
				new Date(start + minutes * 60_000),
				undefined,
				catalog,
			);
		const first = await load(0);
		expect(backend.eventState.getEvents).toHaveBeenCalledOnce();
		expect(first.events.map((item) => item.id)).toEqual(["app:event"]);
		expect([...catalog.keys()]).toEqual(["app"]);
		await load(29);
		expect(backend.eventState.getEvents).toHaveBeenCalledOnce();
		backend.eventState.getEvents = vi.fn(async () => {
			throw new Error("offline");
		}) as never;
		const stale = await load(31);
		expect(backend.eventState.getEvents).toHaveBeenCalledOnce();
		expect(stale.events.map((item) => item.id)).toEqual(["app:event"]);
		expect(
			stale.sections.find((item) => item.kind === "event_favorites")?.state,
		).toBe("ready");
	});
});

describe("native navigation and active runs", () => {
	test("Handoff uses clean paths before stale route and Event selectors", async () => {
		const route = "/café sale/encoded%20path/50%";
		const snapshot = await loadNativeSnapshot(
			fixture([
				event({ default_page_id: "page", route }),
				event({ id: "root", default_page_id: "root-page", is_default: true }),
			]),
			"account-a",
			[],
			true,
		);
		const pathname = "/use/caf%C3%A9%20sale/encoded%2520path/50%25";
		const shared = withNativeActivePage(
			snapshot,
			pathname,
			"id=app&route=%2F&eventId=root&appQuery=tag%3Done%26tag%3Dtwo%26id%3Drecord&sessionId=private&token=secret",
			"https://app.example.com",
		);
		const continued = new URL(shared.activePage?.url ?? "https://missing.test");
		expect(continued.pathname).toBe(pathname);
		expect(readUseRoutePath(continued.pathname)).toBe(route);
		expect([...continued.searchParams.keys()]).toEqual(["id", "appQuery"]);
		expect(appQueryContext(continued.search)._query_param_values?.tag).toEqual([
			"one",
			"two",
		]);
		expect(
			withNativeActivePage(
				snapshot,
				"/use/",
				"id=app&eventId=event",
				"https://app.example.com",
			).activePage?.url,
		).toBe("https://app.example.com/use/?id=app&appQuery=");
		expect(
			withNativeActivePage(
				snapshot,
				"/use",
				"id=app&eventId=event",
				"https://app.example.com",
			).activePage?.url,
		).toBe("https://app.example.com/use?id=app&eventId=event");
	});

	test("Handoff rejects malformed clean paths and duplicate shell selectors", async () => {
		const snapshot = await loadNativeSnapshot(
			fixture([event({ default_page_id: "page", route: "/orders" })]),
			"account-a",
			[],
			true,
		);
		for (const pathname of [
			"/users",
			"/use/unknown",
			"/use/../orders",
			"/use/%2e%2e/orders",
			"/use/a%2Forders",
			"/use//orders",
			"/use/%5Corders",
			"/use/%00",
			"/use/%invalid",
		]) {
			expect(
				withNativeActivePage(
					snapshot,
					pathname,
					"id=app&route=%2Forders&eventId=event",
					"https://app.example.com",
				).activePage,
			).toBeUndefined();
		}
		for (const key of ["id", "eventId", "route", "appQuery"]) {
			const query = new URLSearchParams({
				id: "app",
				eventId: "event",
				route: "/orders",
				appQuery: "tag=a&tag=b",
			});
			query.append(key, query.get(key) ?? "");
			expect(
				withNativeActivePage(
					snapshot,
					"/use/orders",
					query.toString(),
					"https://app.example.com",
				).activePage,
			).toBeUndefined();
		}
		expect(
			withNativeActivePage(
				snapshot,
				"/use/orders",
				"id=other&eventId=event",
				"https://app.example.com",
			).activePage,
		).toBeUndefined();
	});

	test("Handoff preserves known app paths and app-owned repeated query values", async () => {
		const snapshot = await loadNativeSnapshot(
			fixture([event({ default_page_id: "page", route: "/orders/123" })]),
			"account-a",
			[],
			true,
		);
		const href = appRouteUrl(
			"app",
			parseAppRouteTarget("/orders/123?id=customer&tag=one&tag=two", [
				{ name: "raw", value: "A&B + 50% / 東京 #1" },
			]),
		);
		const outer = new URL(href, "https://app.example.com");
		outer.searchParams.set("token", "private shell token");
		outer.searchParams.set("message", "private shell text");
		outer.searchParams.set("eventId", "stale-explicit-event");
		const shared = withNativeActivePage(
			snapshot,
			"/use",
			outer.search,
			"https://app.example.com",
		);
		expect(shared.activePage?.url).toBe(
			`https://app.example.com${pathUseUrl(new URL(href, "https://app.example.com"))}`,
		);
		const continued = new URL(shared.activePage?.url ?? "https://missing.test");
		expect(appQueryContext(continued.search)._query_param_values?.tag).toEqual([
			"one",
			"two",
		]);
		expect(appQueryContext(continued.search)._query_params.raw).toBe(
			"A&B + 50% / 東京 #1",
		);
	});

	test("Handoff skips unknown, non-exposed, cross-app, and unsafe route targets", async () => {
		const snapshot = await loadNativeSnapshot(
			fixture([
				event({ default_page_id: "page", route: "/orders" }),
				event({
					id: "private",
					default_page_id: "private-page",
					route: "/private",
					config: [],
				}),
			]),
			"account-a",
			[],
			true,
		);
		for (const path of [
			"/unknown",
			"/private",
			"https://evil.test",
			"/../orders",
		]) {
			const query = new URLSearchParams({
				id: "app",
				route: path,
				eventId: "event",
				appQuery: "x=one",
			});
			expect(
				withNativeActivePage(
					snapshot,
					"/use",
					query.toString(),
					"https://app.example.com",
				).activePage,
			).toBeUndefined();
		}
		for (const origin of [
			"http://app.example.com",
			"https://user:pass@app.example.com",
			"https://app.example.com/other",
		]) {
			expect(
				withNativeActivePage(snapshot, "/use", "id=app&route=%2Forders", origin)
					.activePage,
			).toBeUndefined();
		}
		expect(
			withNativeActivePage(
				snapshot,
				"/use",
				"id=other&route=%2Forders",
				"https://app.example.com",
			).activePage,
		).toBeUndefined();
	});

	test("default app query keeps its resolved exposed root route for Handoff", async () => {
		const snapshot = await loadNativeSnapshot(
			fixture([event({ default_page_id: "page", is_default: true })]),
			"account-a",
			[],
			true,
		);
		const shared = withNativeActivePage(
			snapshot,
			"/use",
			"id=app&eventId=event&appQuery=tag%3Done%26tag%3Dtwo",
			"https://app.example.com",
		);
		const continued = new URL(shared.activePage?.url ?? "https://missing.test");
		expect(continued.pathname).toBe("/use/");
		expect(continued.searchParams.get("route")).toBeNull();
		expect(appQueryContext(continued.search)._query_param_values?.tag).toEqual([
			"one",
			"two",
		]);
	});

	test("Handoff copies only the selected page route and a configured HTTPS origin", async () => {
		const snapshot = await loadNativeSnapshot(
			fixture([event({ default_page_id: "page" })]),
			"account-a",
			[],
			true,
		);
		const shared = withNativeActivePage(
			snapshot,
			"/use",
			"id=app&eventId=event&message=private&token=secret",
			nativeWebOrigin("app.example.com"),
		);
		expect(shared.activePage).toEqual({
			title: "Translate",
			url: "https://app.example.com/use?id=app&eventId=event",
		});
		expect(
			withNativeActivePage(shared, "/chat", "", shared.webOrigin).activePage,
		).toBeUndefined();
		expect(
			withNativeActivePage(
				snapshot,
				"/use",
				"id=app&eventId=unknown",
				shared.webOrigin,
			).activePage,
		).toBeUndefined();
		expect(
			nativeWebOrigin("https://user:password@app.example.com"),
		).toBeUndefined();
		expect(nativeWebOrigin("http://app.example.com")).toBeUndefined();
	});
	test("deep links cannot execute an Event or cross workspace scope", () => {
		expect(
			nativeNavigationRequest(
				"flow-like://native/run?appId=app&scope=other",
				"account-a",
			),
		).toBeUndefined();
		expect(
			nativeNavigationRequest(
				"flow-like://native/run_event?appId=app&scope=account-a",
				"account-a",
			),
		).toBeUndefined();
		expect(
			nativeNavigationRequest("https://native/home", "account-a"),
		).toBeUndefined();
		expect(
			nativeNavigationRequest(
				"flow-like://native/page?appId=app&eventId=event&scope=account-a",
				"account-a",
			)?.action,
		).toEqual({ kind: "open_event", appId: "app", eventId: "event" });
	});
	test("active executions override matching history and filter other profile apps", async () => {
		const snapshot = await loadNativeSnapshot(fixture(), "account-a", [], true);
		const live = {
			streamId: "stream",
			appId: "app",
			eventId: "event",
			runId: "run",
			title: "Translation",
			startedAt: "2026-09-12T00:00:00Z",
		};
		const merged = withNativeActiveRuns(snapshot, [
			live,
			{ ...live, appId: "other", runId: "other-run" },
		]);
		const items = merged.sections.find(
			(section) => section.kind === "recent_runs",
		)!.items;
		expect(items).toHaveLength(1);
		expect(items[0]).toMatchObject({
			id: "run",
			status: "running",
			action: { runId: "run", eventId: "event" },
		});
		expect(
			snapshot.sections.find((section) => section.kind === "recent_runs")!
				.items[0].status,
		).toBe("Info");
	});
	test("usable app routes are eligible for app discovery without exposing unconfigured Events", async () => {
		const snapshot = await loadNativeSnapshot(
			fixture([event({ default_page_id: "page", config: bytes({}) })]),
			"account-a",
			[],
			true,
		);
		expect(snapshot.apps[0].spotlightEligible).toBe(true);
		expect(snapshot.events).toEqual([]);
	});
});

describe("native selected MCP tools", () => {
	const mcpEvent = () =>
		event({
			event_type: "mcp",
			execution_mode: "Remote" as IEvent["execution_mode"],
			config: bytes({
				native_integration: {
					enabled: true,
					surfaces: ["siri"],
					operation: "translate",
				},
			}),
		});
	const mcpBackend = () => {
		const backend = fixture([mcpEvent()]);
		backend.eventState.invokeMcp = vi.fn(async (_app, _event, method) =>
			method === "tools/list"
				? {
						tools: [
							{
								name: "translate",
								inputSchema: { type: "object", required: ["text"] },
							},
						],
					}
				: { content: [{ type: "text", text: "Done" }] },
		);
		return backend;
	};
	test("exposes one configured operation and dispatches its real input interface", async () => {
		const backend = mcpBackend();
		const snapshot = await loadNativeSnapshot(backend, "account-a", [], true);
		expect(snapshot.events[0].id).toBe("app:event:translate");
		expect(snapshot.events[0].action.operation).toBe("translate");
		const ctx = { ...context(backend), mcp: vi.fn() };
		await dispatchNativeAction(
			{ id: "request", scope: ctx.scope, action: snapshot.events[0].action },
			ctx,
		);
		expect(ctx.mcp).toHaveBeenCalledWith(
			"app",
			expect.objectContaining({ id: "event" }),
			expect.objectContaining({ name: "translate" }),
			"request",
			undefined,
		);
		expect(ctx.execute).not.toHaveBeenCalled();
	});
	test("passes supplied Shortcut input to the tool dialog for review instead of discarding it", async () => {
		const ctx = { ...context(mcpBackend()), mcp: vi.fn() };
		await dispatchNativeAction(
			{
				id: "request",
				scope: ctx.scope,
				action: {
					kind: "run_event",
					appId: "app",
					eventId: "event",
					operation: "translate",
					text: '{"text":"Hello"}',
				},
			},
			ctx,
		);
		expect(ctx.mcp).toHaveBeenCalledWith(
			"app",
			expect.anything(),
			expect.anything(),
			"request",
			'{"text":"Hello"}',
		);
	});
	test("calls only the selected tool with supplied arguments after fresh authority checks", async () => {
		const backend = mcpBackend();
		const result = await executeNativeMcpOperation(
			backend,
			"app",
			"event",
			"translate",
			{ text: "Hello" },
			() => true,
		);
		expect(backend.eventState.getEventAuthoritative).toHaveBeenCalledWith(
			"app",
			"event",
		);
		expect(backend.eventState.invokeMcp).toHaveBeenLastCalledWith(
			"app",
			"event",
			"tools/call",
			{ name: "translate", arguments: { text: "Hello" } },
			expect.any(Function),
		);
		expect(result.status).toBe("ok");
	});
	test("rejects stale selection, removed registration and account changes before tool execution", async () => {
		const backend = mcpBackend();
		await expect(
			executeNativeMcpOperation(
				backend,
				"app",
				"event",
				"delete",
				{},
				() => true,
			),
		).rejects.toThrow("no longer available");
		backend.eventState.invokeMcp = vi.fn(async () => ({ tools: [] }));
		await expect(
			executeNativeMcpOperation(
				backend,
				"app",
				"event",
				"translate",
				{},
				() => true,
			),
		).rejects.toThrow("no longer registered");
		const calls = vi.fn().mockReturnValueOnce(true).mockReturnValue(false);
		await expect(
			executeNativeMcpOperation(
				backend,
				"app",
				"event",
				"translate",
				{},
				calls,
			),
		).rejects.toThrow("account or workspace changed");
		expect(backend.eventState.invokeMcp).toHaveBeenCalledTimes(1);
	});
	test("rejects non-object tool inputs without an API request", async () => {
		const backend = mcpBackend();
		await expect(
			executeNativeMcpOperation(
				backend,
				"app",
				"event",
				"translate",
				[],
				() => true,
			),
		).rejects.toThrow("JSON object");
		expect(backend.eventState.getEventAuthoritative).not.toHaveBeenCalled();
	});
});
describe("native action authority", () => {
	test("merges named Shortcut inputs with defaults and accepts plain text for a single String input", () => {
		const evt = event({
			inputs: [
				{ name: "text", data_type: "String", value_type: "Normal" },
				{ name: "limit", data_type: "Integer", default_value: bytes(5) },
			] as IEvent["inputs"],
		});
		expect(nativeQuickActionPayload(evt, '{"text":"Hello"}')).toEqual({
			text: "Hello",
			limit: 5,
		});
		expect(
			nativeQuickActionPayload(
				event({
					inputs: [
						{ name: "text", data_type: "String", value_type: "Normal" },
					] as IEvent["inputs"],
				}),
				"Translate this",
			),
		).toEqual({ text: "Translate this" });
		expect(() =>
			nativeQuickActionPayload(evt, '{"text":"Hello","unknown":true}'),
		).toThrow("no input named unknown");
		expect(() =>
			nativeQuickActionPayload(evt, '{"text":"Hello","limit":"wrong"}'),
		).toThrow("Integer type");
	});
	test("does not run defaults when a Shortcut supplied incompatible input", async () => {
		const ctx = context(
			fixture([
				event({
					inputs: [
						{ name: "limit", data_type: "Integer", default_value: bytes(5) },
					] as IEvent["inputs"],
				}),
			]),
		);
		await expect(
			dispatchNativeAction(
				{
					id: "request",
					scope: ctx.scope,
					action: {
						kind: "run_event",
						appId: "app",
						eventId: "event",
						text: '{"limit":"wrong"}',
					},
				},
				ctx,
			),
		).rejects.toThrow("Integer type");
		expect(ctx.execute).not.toHaveBeenCalled();
		expect(ctx.navigate).toHaveBeenCalled();
	});
	test("refuses previous-account actions before touching the backend", async () => {
		const ctx = context(fixture());
		await expect(
			dispatchNativeAction(
				{
					id: "request",
					scope: "account-b",
					action: { kind: "run_event", appId: "app", eventId: "event" },
				},
				ctx,
			),
		).rejects.toThrow("different account");
		expect(ctx.backend.appState.getApp).not.toHaveBeenCalled();
	});
	test("rechecks exposure and refuses an Event disabled after widget publication", async () => {
		const ctx = context(fixture([event({ active: false })]));
		await expect(
			dispatchNativeAction(
				{
					id: "request",
					scope: ctx.scope,
					action: { kind: "run_event", appId: "app", eventId: "event" },
				},
				ctx,
			),
		).rejects.toThrow("no longer available");
		expect(ctx.execute).not.toHaveBeenCalled();
	});
	test("executes a configured Quick Action through the Event and opens its interface", async () => {
		const evt = event({
			inputs: [
				{
					name: "limit",
					data_type: "Integer",
					default_value: bytes(5),
				} as never,
			],
		});
		const ctx = context(fixture([evt]));
		await dispatchNativeAction(
			{
				id: "request",
				scope: ctx.scope,
				action: { kind: "run_event", appId: "app", eventId: "event" },
			},
			ctx,
		);
		expect(ctx.execute).toHaveBeenCalledWith(
			"app",
			evt,
			{ limit: 5 },
			"request",
		);
		expect(ctx.navigate).toHaveBeenCalledWith("/use?id=app&eventId=event");
	});
	test("missing required inputs open the existing form without executing", async () => {
		const ctx = context(
			fixture([
				event({
					inputs: [
						{ name: "question", data_type: "String", optional: false } as never,
					],
				}),
			]),
		);
		await dispatchNativeAction(
			{
				id: "request",
				scope: ctx.scope,
				action: { kind: "run_event", appId: "app", eventId: "event" },
			},
			ctx,
		);
		expect(ctx.execute).not.toHaveBeenCalled();
		expect(ctx.navigate).toHaveBeenCalled();
	});
	test("does not let a saved route override the selected page Event", () => {
		expect(
			nativeEventUrl(
				"app",
				event({ route: "/old-alias", default_page_id: "page" }),
				"Hello there",
			),
		).toBe("/use?id=app&eventId=event&message=Hello+there");
	});
	test("null input defaults still require the form", () => {
		expect(
			nativeQuickActionPayload(
				event({
					inputs: [
						{
							name: "required",
							data_type: "String",
							default_value: bytes(null),
						} as never,
					],
				}),
			),
		).toBeNull();
	});
});

describe("typed Shortcut dispatch", () => {
	test.each(["simple_chat", "generic_form", "api", "quick_action"])(
		"%s calls the current Event directly without a navigation trigger",
		async (event_type) => {
			const backend = fixture([event({ event_type })]);
			const ctx = { ...context(backend), callEvent: vi.fn() };
			await dispatchNativeAction(
				{
					id: "response",
					scope: ctx.scope,
					responseMode: "result",
					responseDeadline: new Date(Date.now() + 90_000).toISOString(),
					action: {
						kind: "run_event",
						appId: "app",
						eventId: "event",
						text: "Test",
					},
				},
				ctx,
			);
			expect(ctx.callEvent).toHaveBeenCalledWith(
				"app",
				expect.objectContaining({ id: "event", event_type }),
				"Test",
			);
			expect(ctx.navigate).not.toHaveBeenCalled();
			expect(ctx.execute).not.toHaveBeenCalled();
		},
	);

	test("revoked Siri and Shortcuts visibility prevents direct execution", async () => {
		const backend = fixture([
			event({
				config: bytes({
					native_integration: { enabled: true, surfaces: ["widget"] },
				}),
			}),
		]);
		const ctx = { ...context(backend), callEvent: vi.fn() };
		await expect(
			dispatchNativeAction(
				{
					id: "revoked",
					scope: ctx.scope,
					responseMode: "text",
					action: { kind: "run_event", appId: "app", eventId: "event" },
				},
				ctx,
			),
		).rejects.toThrow("no longer available to Siri and Shortcuts");
		expect(ctx.callEvent).not.toHaveBeenCalled();
	});

	test("expired response requests fail before querying or running the app", async () => {
		const backend = fixture();
		const ctx = { ...context(backend), callEvent: vi.fn() };
		await expect(
			dispatchNativeAction(
				{
					id: "expired",
					scope: ctx.scope,
					responseMode: "result",
					responseDeadline: new Date(0).toISOString(),
					action: { kind: "run_event", appId: "app", eventId: "event" },
				},
				ctx,
			),
		).rejects.toThrow("expired");
		expect(ctx.callEvent).not.toHaveBeenCalled();
		expect(backend.appState.getApp).not.toHaveBeenCalled();
	});
});
