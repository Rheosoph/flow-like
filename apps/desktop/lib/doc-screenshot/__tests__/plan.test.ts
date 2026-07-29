import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";

import {
	loadDocScreenshotHttpFixture,
	loadDocScreenshotPlan,
	validateDocScreenshotHttpFixture,
	validateDocScreenshotPlan,
	validateDocScreenshotTauriFixture,
} from "../plan";
import {
	DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
	DOC_SCREENSHOT_PLAN_SCHEMA,
	DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA,
} from "../types";

function planWithSteps(
	steps: unknown[],
	overrides: Record<string, unknown> = {},
) {
	return {
		schema: DOC_SCREENSHOT_PLAN_SCHEMA,
		app: "desktop",
		outputDir: "tmp/doc-screenshots",
		scenarios: [
			{
				name: "example",
				path: "/onboarding",
				steps,
			},
		],
		...overrides,
	};
}

describe("document screenshot plan validation", () => {
	test("validates the checked-in onboarding example", async () => {
		const examplePath = fileURLToPath(
			new URL("../examples/onboarding.plan.json", import.meta.url),
		);
		const plan = await loadDocScreenshotPlan(examplePath);

		expect(plan.schema).toBe(DOC_SCREENSHOT_PLAN_SCHEMA);
		expect(plan.defaults.viewport).toEqual({
			width: 1624,
			height: 1060,
			deviceScaleFactor: 2,
		});
		expect(plan.scenarios).toHaveLength(1);
		expect(
			plan.scenarios[0]?.steps
				.filter((step) => step.type === "capture")
				.map((step) => step.name),
		).toEqual(["overview", "selected", "complete"]);
	});

	test("rejects capture output traversal", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([
					{
						type: "capture",
						name: "escape",
						output: "../escape.png",
					},
				]),
			),
		).toThrow("must stay inside outputDir");
	});

	test("rejects duplicate capture names", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([
					{ type: "capture", name: "same" },
					{ type: "capture", name: "same" },
				]),
			),
		).toThrow("Duplicate capture name: same");
	});

	test("rejects an element capture without a selector", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([{ type: "capture", name: "card", mode: "element" }]),
			),
		).toThrow("selector is required for element capture");
	});

	test("rejects viewports above the output pixel limit", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([{ type: "capture", name: "oversized" }], {
					defaults: {
						viewport: {
							width: 7680,
							height: 7680,
							deviceScaleFactor: 2,
						},
					},
				}),
			),
		).toThrow("exceeds the 100 megapixel capture limit");
	});

	test("validates drag controls and click keyboard modifiers", () => {
		const plan = validateDocScreenshotPlan(
			planWithSteps([
				{
					type: "click",
					selector: ".react-flow__node",
					index: 1,
					modifiers: ["Control", "Shift"],
				},
				{
					type: "drag",
					selector: ".react-flow__node",
					index: 1,
					targetSelector: ".react-flow__pane",
					targetIndex: 0,
					steps: 40,
					button: "left",
					release: false,
				},
				{ type: "capture", name: "after-drag" },
			]),
		);

		expect(plan.scenarios[0]?.steps.slice(0, 2)).toEqual([
			{
				type: "click",
				selector: ".react-flow__node",
				index: 1,
				button: undefined,
				clickCount: undefined,
				modifiers: ["Control", "Shift"],
			},
			{
				type: "drag",
				selector: ".react-flow__node",
				index: 1,
				targetSelector: ".react-flow__pane",
				targetIndex: 0,
				steps: 40,
				button: "left",
				release: false,
			},
		]);
	});

	test.each([0, 101, 1.5])("rejects unsafe drag step count %s", (steps) => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([
					{
						type: "drag",
						selector: "#source",
						targetSelector: "#destination",
						steps,
					},
					{ type: "capture", name: "after-drag" },
				]),
			),
		).toThrow("steps must be an integer from 1 to 100");
	});

	test.each([
		[["Control", "Control"], "cannot contain duplicate"],
		[["Control", "CapsLock"], "must be one of"],
		[[], "must contain from 1 to 4"],
	])("rejects invalid click modifiers %j", (modifiers, error) => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([
					{ type: "click", selector: "#source", modifiers },
					{ type: "capture", name: "after-click" },
				]),
			),
		).toThrow(error);
	});

	test("requires a drag target selector", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([
					{ type: "drag", selector: "#source" },
					{ type: "capture", name: "after-drag" },
				]),
			),
		).toThrow("targetSelector is required");
	});

	test("requires a held drag to be followed immediately by a capture", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([
					{
						type: "drag",
						selector: "#source",
						targetSelector: "#destination",
						release: false,
					},
					{ type: "delay", ms: 10 },
					{ type: "capture", name: "after-drag" },
				]),
			),
		).toThrow("release false must be followed immediately by a capture step");
	});

	test.each([
		[{ type: "click", selector: "#source", modifier: ["Control"] }, "modifier"],
		[
			{
				type: "drag",
				selector: "#source",
				targetSelector: "#destination",
				realease: false,
			},
			"realease",
		],
	])("rejects unknown step fields", (step, field) => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([step, { type: "capture", name: "after-action" }]),
			),
		).toThrow(`${field} is not supported`);
	});

	test("rejects unsupported actions", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([
					{ type: "swipe", selector: "#source", target: "#destination" },
					{ type: "capture", name: "after-swipe" },
				]),
			),
		).toThrow("type is not supported: swipe");
	});
});

describe("Tauri screenshot fixture validation", () => {
	test("defaults strict fixture handling to true", () => {
		const fixture = validateDocScreenshotTauriFixture({
			schema: DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA,
			responses: {
				get_profile: { id: "docs", enabled: true },
			},
		});

		expect(fixture.strict).toBe(true);
		expect(fixture.responses).toEqual({
			get_profile: { id: "docs", enabled: true },
		});
	});

	test.each([
		["undefined", undefined],
		["NaN", Number.NaN],
		["bigint", BigInt(1)],
	])("rejects a non-JSON %s response", (_label, response) => {
		expect(() =>
			validateDocScreenshotTauriFixture({
				schema: DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA,
				responses: { invalid: response },
			}),
		).toThrow("fixture.responses.invalid is not JSON-serializable");
	});
});

describe("HTTP screenshot fixture validation", () => {
	test("loads and normalizes the checked-in reference fixture", async () => {
		const fixturePath = fileURLToPath(
			new URL("../fixtures/docs-reference.http.json", import.meta.url),
		);
		const fixture = await loadDocScreenshotHttpFixture(fixturePath);

		expect(fixture.schema).toBe(DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA);
		expect(fixture.strict).toBe(true);
		expect(fixture.routes).toHaveLength(10);
		expect(fixture.blockedOrigins).toEqual([
			"https://o4507505692901376.ingest.de.sentry.io",
			"https://a.basemaps.cartocdn.com",
			"https://b.basemaps.cartocdn.com",
			"https://c.basemaps.cartocdn.com",
			"https://img.youtube.com",
			"https://www.youtube-nocookie.com",
			"https://open.spotify.com",
			"https://platform.twitter.com",
			"https://www.redditmedia.com",
			"https://opengraph.githubassets.com",
			"https://www.linkedin.com",
			"https://www.google.com",
		]);
		expect(fixture.blockedEndpoints).toEqual([
			"http://localhost:8080/api/v1/og",
		]);
		expect(fixture.routes[1]?.request).toEqual({
			method: "GET",
			url: "http://localhost:8080/api/v1/auth/openid",
			body: undefined,
		});
		expect(fixture.routes[1]?.response.headers).toEqual({
			"access-control-allow-origin": "*",
		});
	});

	test("defaults strict handling, status, and headers", () => {
		const fixture = validateDocScreenshotHttpFixture({
			schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
			routes: [
				{
					request: {
						method: "GET",
						url: "https://api.example.test/config",
					},
					response: {
						json: { enabled: true },
					},
				},
			],
		});

		expect(fixture.strict).toBe(true);
		expect(fixture.blockedOrigins).toEqual([]);
		expect(fixture.blockedEndpoints).toEqual([]);
		expect(fixture.routes[0]?.response).toEqual({
			status: 200,
			headers: {},
			body: undefined,
			json: { enabled: true },
		});
	});

	test("validates blocked endpoints without accepting query strings", () => {
		const route = {
			request: {
				method: "GET",
				url: "https://api.example.test/config",
			},
			response: {},
		};
		expect(() =>
			validateDocScreenshotHttpFixture({
				schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
				blockedEndpoints: [
					"https://api.example.test/og?url=https%3A%2F%2Fexample.test",
				],
				routes: [route],
			}),
		).toThrow("without query or fragment");
		expect(() =>
			validateDocScreenshotHttpFixture({
				schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
				blockedEndpoints: [
					"https://api.example.test/og",
					"https://api.example.test/og",
				],
				routes: [route],
			}),
		).toThrow("cannot contain duplicate endpoints");
	});

	test("validates blocked origins without accepting paths or duplicates", () => {
		const route = {
			request: {
				method: "GET",
				url: "https://api.example.test/config",
			},
			response: {},
		};
		expect(() =>
			validateDocScreenshotHttpFixture({
				schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
				blockedOrigins: ["https://telemetry.example.test/envelope"],
				routes: [route],
			}),
		).toThrow("absolute HTTP or HTTPS origin");
		expect(() =>
			validateDocScreenshotHttpFixture({
				schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
				blockedOrigins: [
					"https://telemetry.example.test",
					"https://telemetry.example.test/",
				],
				routes: [route],
			}),
		).toThrow("cannot contain duplicate origins");
	});

	test.each([
		[
			{
				method: "get",
				url: "https://api.example.test/config",
			},
			"uppercase HTTP method",
		],
		[
			{
				method: "GET",
				url: "/api/config",
			},
			"absolute HTTP or HTTPS URL",
		],
		[
			{
				method: "GET",
				url: "https://user:password@api.example.test/config",
			},
			"cannot contain credentials",
		],
	])("rejects an invalid exact request match %#", (request, error) => {
		expect(() =>
			validateDocScreenshotHttpFixture({
				schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
				routes: [{ request, response: {} }],
			}),
		).toThrow(error);
	});

	test("rejects duplicate exact request matches", () => {
		const route = {
			request: {
				method: "POST",
				url: "https://api.example.test/config",
				body: "{}",
			},
			response: { status: 200 },
		};
		expect(() =>
			validateDocScreenshotHttpFixture({
				schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
				routes: [route, structuredClone(route)],
			}),
		).toThrow("overlaps an earlier exact request match");
	});

	test("rejects a body-agnostic route that overlaps a body-specific route", () => {
		expect(() =>
			validateDocScreenshotHttpFixture({
				schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
				routes: [
					{
						request: {
							method: "POST",
							url: "https://api.example.test/envelope",
							body: "{}",
						},
						response: {},
					},
					{
						request: {
							method: "POST",
							url: "https://api.example.test/envelope",
						},
						response: {},
					},
				],
			}),
		).toThrow("overlaps an earlier exact request match");
	});

	test("rejects ambiguous response bodies and unknown fields", () => {
		expect(() =>
			validateDocScreenshotHttpFixture({
				schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
				routes: [
					{
						request: {
							method: "GET",
							url: "https://api.example.test/config",
						},
						response: {
							body: "{}",
							json: {},
						},
					},
				],
			}),
		).toThrow("cannot define both body and json");

		expect(() =>
			validateDocScreenshotHttpFixture({
				schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
				routes: [
					{
						request: {
							method: "GET",
							url: "https://api.example.test/config",
							methodPattern: "G.*",
						},
						response: {},
					},
				],
			}),
		).toThrow("methodPattern is not supported");
	});
});
