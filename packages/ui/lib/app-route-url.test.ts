import { describe, expect, test } from "bun:test";
import {
	appQueryContext,
	appRouteUrl,
	parseAppRouteTarget,
	readAppQuery,
	setAppQueryParam,
} from "./app-route-url";

describe("app-owned native routes", () => {
	test("encodes raw structured values once and preserves raw URL query semantics", () => {
		const value = " spaces café 東京 50% + & # ? = %2B";
		const target = parseAppRouteTarget(
			"orders/123?tab=details&tag=one&plus=%2B&space=hello+world&url=a?b=c",
			[
				{ name: "tag", value: "two" },
				{ name: "raw", value },
			],
		);
		const url = new URL(
			appRouteUrl("chosen + app", target),
			"https://app.test",
		);
		expect(url.pathname).toBe("/use");
		expect(url.searchParams.get("id")).toBe("chosen + app");
		expect(url.searchParams.get("route")).toBe("/orders/123");
		const query = readAppQuery(url.search);
		expect(query.get("raw")).toBe(value);
		expect(query.getAll("tag")).toEqual(["one", "two"]);
		expect(query.get("plus")).toBe("+");
		expect(query.get("space")).toBe("hello world");
		expect(query.get("url")).toBe("a?b=c");
	});

	test("keeps reserved names, literal underscores and duplicate params inside the app", () => {
		const target = parseAppRouteTarget(
			"/settings?id=other&eventId=wrong&route=/flow",
			[
				{ name: "id", value: "second" },
				{ name: "_id", value: "literal underscore" },
				{ name: "appQuery", value: "opaque" },
				{ name: "__proto__", value: "plain data" },
			],
		);
		const url = new URL(appRouteUrl("chosen", target), "https://app.test");
		expect(url.searchParams.get("id")).toBe("chosen");
		expect(url.searchParams.get("eventId")).toBeNull();
		expect(url.searchParams.get("route")).toBe("/settings");
		const context = appQueryContext(url.search);
		expect(context._query_params).toEqual({
			id: "second",
			eventId: "wrong",
			route: "/flow",
			_id: "literal underscore",
			appQuery: "opaque",
			["__proto__"]: "plain data",
		});
		expect(context._query_param_values?.id).toEqual(["other", "second"]);
		expect(context._query_param_values?.__proto__).toEqual(["plain data"]);
		expect(context._query_params_format).toBe("app");
	});

	test("preserves authored route characters and canonical route matching", () => {
		for (const path of [
			"/café sale/50%",
			"/encoded%20path",
			"/reports/12:30",
			"/use",
		]) {
			const target = parseAppRouteTarget(`${path}/`);
			expect(
				new URL(
					appRouteUrl("app", target),
					"https://app.test",
				).searchParams.get("route"),
			).toBe(path);
		}
		expect(appRouteUrl("app", parseAppRouteTarget())).toBe("/use?id=app");
	});

	test("rejects shell escapes, external URLs, traversal and raw fragments", () => {
		for (const path of [
			"https://evil.test",
			"javascript:alert(1)",
			"//evil.test",
			"\\evil.test",
			"/%2Fexample.com",
			"/%5cevil",
			"/../flow",
			"/.%2e/flow",
			"/%2e%2e/flow",
			"/%2e%2e/%invalid",
			"/path#fragment",
			"/path\n",
			"/%00",
			"/reports%0A%",
		])
			expect(() => parseAppRouteTarget(path)).toThrow();
	});

	test("validates UTF-8 limits, shape, and decoded query controls", () => {
		for (const args of [
			[`/${"é".repeat(2048)}`],
			["/", Array.from({ length: 33 }, () => ({ name: "x", value: "y" }))],
			["/", [{ name: "", value: "x" }]],
			["/", [{ name: "x", value: 1 }]],
			["/", [{ name: "é".repeat(129), value: "x" }]],
			["/", [{ name: "x", value: "é".repeat(2049) }]],
			["/?x=%00"],
			["/", [{ name: "x", value: "line\n" }]],
			["/", { x: "y" }],
			[true],
		])
			expect(() => parseAppRouteTarget(args[0], args[1])).toThrow();
	});

	test("query updates preserve shell identity and the remaining duplicate values", () => {
		const url = new URL(
			appRouteUrl("chosen", parseAppRouteTarget("/orders?tag=a&tag=b")),
			"https://app.test",
		);
		setAppQueryParam(url, "id", "app-owned");
		expect(url.searchParams.get("id")).toBe("chosen");
		expect(readAppQuery(url.search).get("id")).toBe("app-owned");
		expect(readAppQuery(url.search).getAll("tag")).toEqual(["a", "b"]);
		setAppQueryParam(url, "id");
		expect(url.searchParams.get("id")).toBe("chosen");
		expect(readAppQuery(url.search).has("id")).toBe(false);
		const empty = new URL(
			appRouteUrl("chosen", parseAppRouteTarget("/orders")),
			"https://app.test",
		);
		setAppQueryParam(empty, "id", "new order");
		expect(empty.searchParams.get("id")).toBe("chosen");
		expect(readAppQuery(empty.search).get("id")).toBe("new order");
	});

	test("keeps legacy flattened notification query behavior", () => {
		expect(
			appQueryContext("?id=app&_id=custom&route=%2Fmail&tag=one&tag=two"),
		).toEqual({
			_query_params: { id: "app", _id: "custom", route: "/mail", tag: "two" },
		});
	});
});
