import { describe, expect, test } from "bun:test";
import {
	HOSTING_BLOCKER_FIX,
	getHostedFrontendKind,
	getHostedFrontendUrl,
	getHostedRoute,
	getHostingBlockers,
	parseFrontendHosting,
} from "./frontend-hosting";
import { IEventExecutionMode, IEventExposure } from "./schema/flow/event";

describe("hosted frontend access configuration", () => {
	test("does not enable hosting for existing or malformed configuration", () => {
		for (const config of [
			undefined,
			null,
			{},
			{ frontend_hosting: true },
			{ frontend_hosting: [] },
			{ frontend_hosting: { enabled: "true", auth_proxy: "false" } },
			{ frontend_hosting: { enabled: true, auth_proxy: "true" } },
			{ frontend_hosting: { enabled: true, allow_anonymous: "true" } },
			{ frontend_hosting: { enabled: true, allow_anonymous: null } },
			{ frontend_hosting: { enabled: true, allow_anonymous: 1 } },
			{ frontend_hosting: { enabled: true, unexpected: true } },
		]) {
			expect(parseFrontendHosting(config)).toEqual({
				enabled: false,
				allow_anonymous: false,
				auth_proxy: true,
			});
		}
	});

	test("enabling hosting requires sign-in until anonymous access is explicitly allowed", () => {
		for (const hosting of [
			{ enabled: true },
			{ enabled: true, auth_proxy: false },
			{ enabled: true, auth_proxy: true },
			{ enabled: true, allow_anonymous: false },
			{ enabled: true, allow_anonymous: false, auth_proxy: false },
			{ enabled: true, allow_anonymous: true, auth_proxy: true },
		]) {
			expect(parseFrontendHosting({ frontend_hosting: hosting })).toEqual({
				enabled: true,
				allow_anonymous: false,
				auth_proxy: true,
			});
		}
	});

	test("allows anonymous access only with both explicit opt-ins", () => {
		for (const hosting of [
			{ enabled: true, allow_anonymous: true },
			{ enabled: true, allow_anonymous: true, auth_proxy: false },
		]) {
			expect(
				parseFrontendHosting({
					frontend_hosting: hosting,
				}),
			).toEqual({ enabled: true, allow_anonymous: true, auth_proxy: false });
		}
		for (const hosting of [
			{ allow_anonymous: true },
			{ enabled: false, allow_anonymous: true, auth_proxy: false },
		]) {
			expect(parseFrontendHosting({ frontend_hosting: hosting })).toEqual({
				enabled: false,
				allow_anonymous: false,
				auth_proxy: true,
			});
		}
	});

	test("routes page targets before considering their trigger type", () => {
		expect(
			getHostedFrontendKind({
				event_type: "simple_chat",
				default_page_id: "custom-page",
			}),
		).toBe("u");
		for (const event_type of ["generic_form", "quick_action"]) {
			expect(getHostedFrontendKind({ event_type })).toBe("f");
		}
		expect(getHostedFrontendKind({ event_type: "simple_chat" })).toBe("c");
		for (const event_type of ["rest", "mcp", "cron", "unknown"]) {
			expect(getHostedFrontendKind({ event_type })).toBeNull();
		}
	});

	test("addresses the app and puts the route in the path", () => {
		expect(getHostedFrontendUrl("https://api.example.com/", "app-1", "/")).toBe(
			"https://api.example.com/a/app-1",
		);
		expect(
			getHostedFrontendUrl("https://api.example.com", "app-1", "topic/"),
		).toBe("https://api.example.com/a/app-1/topic");
		expect(
			getHostedFrontendUrl("http://localhost:8080", "app/1", "/a b/c?x=1#y"),
		).toBe("http://localhost:8080/a/app%2F1/a%20b/c");
	});

	test("reads the route an Event answers like the server does", () => {
		expect(getHostedRoute({ route: "/config/" })).toBe("/config");
		expect(getHostedRoute({ route: "feed", is_default: true })).toBe("/feed");
		expect(getHostedRoute({ route: " ", is_default: true })).toBe("/");
		expect(getHostedRoute({ route: null, is_default: false })).toBeNull();
		expect(getHostedRoute({})).toBeNull();
	});

	test("names every requirement the server checks before serving a hosted link", () => {
		const live = {
			active: true,
			execution_mode: IEventExecutionMode.Remote,
			exposure: IEventExposure.Public,
		};
		expect(getHostingBlockers(live, "/")).toEqual([]);
		expect(getHostingBlockers({ ...live, exposure: undefined }, "/x")).toEqual(
			[],
		);
		expect(getHostingBlockers(live, null)).toEqual(["no_route"]);
		expect(
			getHostingBlockers(
				{ ...live, execution_mode: IEventExecutionMode.Local },
				"/",
			),
		).toEqual(["local_execution"]);
		expect(
			getHostingBlockers({ ...live, execution_mode: undefined }, "/"),
		).toEqual(["local_execution"]);
		const broken = {
			active: false,
			execution_mode: IEventExecutionMode.Local,
			exposure: IEventExposure.Internal,
		};
		const blockers = getHostingBlockers(broken, null);
		expect(blockers).toEqual([
			"no_route",
			"inactive",
			"local_execution",
			"internal_exposure",
		]);
		expect(HOSTING_BLOCKER_FIX.no_route).toBeUndefined();
		const fixed = Object.assign(
			{ ...broken },
			...blockers.map((blocker) => HOSTING_BLOCKER_FIX[blocker] ?? {}),
		);
		expect(getHostingBlockers(fixed, "/")).toEqual([]);
	});
});
