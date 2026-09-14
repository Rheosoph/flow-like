import { afterEach, describe, expect, test } from "bun:test";
import { appRouteUrl, parseAppRouteTarget } from "../../lib/app-route-url";
import {
	buildFrontendContextPayload,
	compactWorkflowPayload,
} from "./workflow-payload";

describe("compactWorkflowPayload", () => {
	test("preserves null values and array positions", () => {
		expect(
			compactWorkflowPayload({
				cleared: null,
				values: [1, null, undefined, 4],
				omitted: undefined,
			}),
		).toEqual({
			cleared: null,
			values: [1, null, null, 4],
		});
	});

	test("removes transient upload previews", () => {
		expect(
			compactWorkflowPayload({
				file: {
					name: "report.pdf",
					size: 42,
					type: "application/pdf",
					dataUrl: "data:application/pdf;base64,large-preview",
					backendUrl: "signed://report",
				},
			}),
		).toEqual({
			file: {
				name: "report.pdf",
				size: 42,
				type: "application/pdf",
				backendUrl: "signed://report",
			},
		});
	});

	test("keeps unrelated dataUrl fields", () => {
		expect(
			compactWorkflowPayload({
				custom: { dataUrl: "data:text/plain,meaningful" },
			}),
		).toEqual({ custom: { dataUrl: "data:text/plain,meaningful" } });
	});
});

describe("buildFrontendContextPayload", () => {
	const originalWindow = (globalThis as { window?: unknown }).window;

	afterEach(() => {
		if (originalWindow === undefined) {
			(globalThis as { window?: unknown }).window = undefined;
		} else {
			(globalThis as { window?: unknown }).window = originalWindow;
		}
	});

	function stubLocation(pathname: string, search: string) {
		(globalThis as { window?: unknown }).window = {
			location: { pathname, search },
		};
	}

	test("carries isolated native app query data into button and widget Event payloads", () => {
		const href = appRouteUrl(
			"app",
			parseAppRouteTarget("/orders?id=order&tag=a&tag=b", [
				{ name: "value", value: "é + 50% & #" },
			]),
		);
		stubLocation("/use", new URL(href, "https://app.test").search);
		expect(buildFrontendContextPayload("orders-page", {}, {})).toMatchObject({
			_route: "/orders",
			_query_params: { id: "order", tag: "b", value: "é + 50% & #" },
			_query_params_format: "app",
			_query_param_values: { tag: ["a", "b"] },
		});
	});

	test("keeps the page id distinct from the route and query params", () => {
		stubLocation("/use", "?id=app-1&route=%2Fmail&mailid=42");
		expect(
			buildFrontendContextPayload(
				"page-mail",
				{ theme: "dark" },
				{ tab: "inbox" },
			),
		).toEqual({
			_route: "/use",
			_query_params: { id: "app-1", route: "/mail", mailid: "42" },
			_page_id: "page-mail",
			_global_state: { theme: "dark" },
			_page_state: { tab: "inbox" },
		});
	});

	test("falls back to defaults without a page id or state", () => {
		stubLocation("/use", "");
		expect(buildFrontendContextPayload(null, undefined, undefined)).toEqual({
			_route: "/use",
			_query_params: {},
			_page_id: "default",
			_global_state: {},
			_page_state: {},
		});
	});

	test("stays inert during server rendering", () => {
		(globalThis as { window?: unknown }).window = undefined;
		expect(buildFrontendContextPayload("/use", {}, {})).toEqual({
			_route: "",
			_query_params: {},
			_page_id: "/use",
			_global_state: {},
			_page_state: {},
		});
	});
});
