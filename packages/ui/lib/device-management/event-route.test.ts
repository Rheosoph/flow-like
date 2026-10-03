import { expect, test } from "bun:test";
import {
	type RouteClaimEvent,
	type RouteMethod,
	deviceRoute,
	routeClaims,
	routeConflicts,
} from "./event-route";

/* The route table of run-more-2-design §1.2; the agent's test carries the same literals. */

const LONG_PATH = `/${"a".repeat(2048)}`;

test("a config gives one route, or the reason a device serves none", () => {
	const cases: [unknown, string][] = [
		[{ path: "/orders", method: "get" }, "GET /orders"],
		[{ path: "orders" }, "POST /orders"],
		[{ path_suffix: "/a/b", method: "PUT" }, "PUT /a/b"],
		[
			{
				sink_type: "http",
				method: "GET",
				path: "/cm1abc",
				public_endpoint: true,
				auth_token: "x",
			},
			"GET /cm1abc",
		],
		[{}, "route_missing"],
		[{ path: 7 }, "route_missing"],
		[[], "route_missing"],
		[{ path: "/a b" }, "route_invalid"],
		[{ path: "/a?x=1" }, "route_invalid"],
		[{ path: "/a/../b" }, "route_invalid"],
		[{ path: "/ä" }, "route_invalid"],
		[{ path: "/x", method: "TRACE" }, "route_invalid"],
		[{ path: LONG_PATH }, "route_invalid"],
		[{ path: "/services" }, "route_reserved"],
		[{ path: "/ui/x" }, "route_reserved"],
		[{ path: "/channels/1" }, "route_reserved"],
		[{ path: "/servicesx" }, "POST /servicesx"],
		[{ path: "/runner" }, "POST /runner"],
		[{ path: "/run/evt_a" }, "POST /run/evt_a"],
	];
	for (const [config, expected] of cases) {
		const result = deviceRoute(config);
		expect([
			config,
			result.ok
				? `${result.route.method} ${result.route.path}`
				: result.problem,
		]).toEqual([config, expected]);
	}
	expect(LONG_PATH.length).toBe(2049);
});

test("only `path` with a leading slash and a method is the form older agents read", () => {
	const strict = (config: unknown) => {
		const result = deviceRoute(config);
		if (!result.ok)
			throw new Error(`expected a route for ${JSON.stringify(config)}`);
		return result.strict;
	};
	expect(strict({ path: "/orders", method: "get" })).toBe(true);
	expect(strict({ path: "orders" })).toBe(false);
	expect(strict({ path: "/orders" })).toBe(false);
	expect(strict({ path_suffix: "/a/b", method: "PUT" })).toBe(false);
});

test("a refused route says what was written, short, and never more than the device read", () => {
	expect(deviceRoute({ path: "/x", method: "TRACE" })).toEqual({
		ok: false,
		problem: "route_invalid",
		detail: "TRACE",
	});
	expect(deviceRoute({ path: "/x", method: 7 })).toEqual({
		ok: false,
		problem: "route_invalid",
		detail: "7",
	});
	expect(deviceRoute({ path: "/services" })).toEqual({
		ok: false,
		problem: "route_reserved",
		detail: "/services",
	});
	const long = deviceRoute({ path: LONG_PATH });
	expect(long.ok ? null : long.detail?.length).toBe(64);
	expect(deviceRoute({ path: "/x", method: null })).toMatchObject({
		ok: true,
		route: { method: "POST", path: "/x" },
	});
	expect(deviceRoute(null)).toEqual({ ok: false, problem: "route_missing" });
	expect(deviceRoute({ path: 7, path_suffix: "a" })).toMatchObject({
		ok: true,
		route: { method: "POST", path: "/a" },
	});
});

const form: RouteClaimEvent = { id: "evt_form", event_type: "generic_form" };
const endpoint = (method: RouteMethod, path: string, id = "evt_api") => ({
	id,
	event_type: "api",
	route: { method, path },
});

test("an Endpoint at the run route of a form of the same service conflicts only where that route exists", () => {
	const events = [form, endpoint("POST", "/run/evt_form")];
	expect(routeConflicts(events, true)).toEqual([
		{
			method: "POST",
			path: "/run/evt_form",
			eventIds: ["evt_form", "evt_api"],
		},
	]);
	expect(routeConflicts(events, false)).toEqual([]);
	expect(routeConflicts([endpoint("POST", "/run/evt_form")], true)).toEqual([]);
	expect(
		routeConflicts([form, endpoint("GET", "/run/evt_form")], true),
	).toEqual([]);
});

test("two events on one method and path, a chat route or a Page endpoint conflict", () => {
	expect(
		routeConflicts(
			[
				endpoint("GET", "/orders", "evt_a"),
				endpoint("GET", "/orders", "evt_b"),
				endpoint("POST", "/orders", "evt_c"),
			],
			true,
		),
	).toEqual([{ method: "GET", path: "/orders", eventIds: ["evt_a", "evt_b"] }]);
	expect(
		routeConflicts(
			[
				{ id: "evt_chat", event_type: "simple_chat" },
				endpoint("POST", "/chat/evt_chat"),
			],
			true,
		),
	).toEqual([
		{
			method: "POST",
			path: "/chat/evt_chat",
			eventIds: ["evt_chat", "evt_api"],
		},
	]);
	expect(
		routeConflicts(
			[
				{ id: "evt_page", event_type: "api", default_page_id: "page_1" },
				endpoint("GET", "/pages/evt_page/bootstrap"),
			],
			true,
		),
	).toEqual([
		{
			method: "GET",
			path: "/pages/evt_page/bootstrap",
			eventIds: ["evt_page", "evt_api"],
		},
	]);
});

test("each kind claims its own routes on the listener", () => {
	expect(
		routeClaims(
			{ id: "p", event_type: "generic_form", default_page_id: "x" },
			false,
		),
	).toEqual([
		{ method: "GET", path: "/pages/p/bootstrap" },
		{ method: "POST", path: "/pages/p/invoke" },
	]);
	expect(routeClaims({ id: "q", event_type: "quick_action" }, true)).toEqual([
		{ method: "POST", path: "/run/q" },
	]);
	expect(routeClaims({ id: "q", event_type: "quick_action" }, false)).toEqual(
		[],
	);
	for (const event_type of ["cron", "daemon", "rest", "telegram", "discord"])
		expect(
			routeClaims(
				{ id: "e", event_type, route: { method: "GET", path: "/x" } },
				true,
			),
		).toEqual([]);
	expect(routeClaims({ id: "h", event_type: "http" }, true)).toEqual([]);
});
