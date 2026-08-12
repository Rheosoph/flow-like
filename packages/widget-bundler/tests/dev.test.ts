import { describe, expect, test } from "bun:test";
import { utimesSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { WidgetContract } from "../src/contract-types";
import {
	ContractCache,
	buildContractPayload,
	handleContractRequest,
	resolveWidgetConfigPath,
} from "../src/dev/contract-endpoint";
import { derivePropsFormModel } from "../src/dev/form-model";
import { type HarnessWidget, harnessHtml } from "../src/dev/harness-html";
import { widgetEntryUrl } from "../src/dev/server";
import type { ExtractResult } from "../src/extract";
import { HELLO_WIDGET_CONFIG, makeProjectFixture } from "./helpers";

const CONTRACT: WidgetContract = {
	contractVersion: 1,
	id: "demo",
	inputs: {
		title: { type: "string", description: "Headline", default: "Sales" },
		variant: { type: "enum", choices: ["bar", "line"], default: "bar" },
		limit: { type: "integer", min: 1, max: 500, default: 50 },
		ratio: { type: "number" },
		visible: { type: "boolean", default: true, optional: true },
		rows: { type: "json", schema: { type: "array" }, default: [] },
	},
	events: { pointSelected: { payloadSchema: { type: "object" } } },
	queries: { getValue: { argsSchema: null, resultSchema: { type: "string" } } },
	sizing: { defaultHeight: 320, resizable: true },
};

describe("derivePropsFormModel", () => {
	const fields = derivePropsFormModel(CONTRACT);
	const byKey = new Map(fields.map((field) => [field.key, field]));

	test("maps every input to one field in contract order", () => {
		expect(fields.map((field) => field.key)).toEqual([
			"title",
			"variant",
			"limit",
			"ratio",
			"visible",
			"rows",
		]);
	});

	test("string input becomes a text control with description and default", () => {
		expect(byKey.get("title")).toEqual({
			key: "title",
			label: "title",
			description: "Headline",
			optional: false,
			default: "Sales",
			control: { kind: "text" },
		});
	});

	test("enum input becomes a select with choices", () => {
		expect(byKey.get("variant")?.control).toEqual({
			kind: "select",
			choices: ["bar", "line"],
		});
		expect(byKey.get("variant")?.default).toBe("bar");
	});

	test("integer input becomes a number control with bounds", () => {
		expect(byKey.get("limit")?.control).toEqual({
			kind: "number",
			integer: true,
			min: 1,
			max: 500,
		});
	});

	test("number input without bounds omits min/max", () => {
		expect(byKey.get("ratio")?.control).toEqual({
			kind: "number",
			integer: false,
		});
		expect(byKey.get("ratio")?.default).toBeUndefined();
	});

	test("boolean input becomes a checkbox and keeps optional", () => {
		expect(byKey.get("visible")?.control).toEqual({ kind: "checkbox" });
		expect(byKey.get("visible")?.optional).toBe(true);
	});

	test("json input carries its schema", () => {
		expect(byKey.get("rows")?.control).toEqual({
			kind: "json",
			schema: { type: "array" },
		});
	});
});

describe("buildContractPayload", () => {
	const extracted: ExtractResult = {
		contract: CONTRACT,
		config: {
			id: "demo",
			name: "Demo",
			description: "",
			fixtures: { empty: { rows: [] }, loaded: { title: "Q3", limit: 10 } },
		},
		warnings: ["input 'x' has no default"],
	};
	const payload = buildContractPayload(extracted);

	test("exposes fixtures and warnings", () => {
		expect(payload.fixtures).toEqual({
			empty: { rows: [] },
			loaded: { title: "Q3", limit: 10 },
		});
		expect(payload.warnings).toEqual(["input 'x' has no default"]);
	});

	test("contract is canonicalized and form model follows its key order", () => {
		expect(Object.keys(payload.contract.inputs)).toEqual([
			"limit",
			"ratio",
			"rows",
			"title",
			"variant",
			"visible",
		]);
		expect(payload.formModel.map((field) => field.key)).toEqual(
			Object.keys(payload.contract.inputs),
		);
	});

	test("defaults to empty fixtures when the config declares none", () => {
		const bare = buildContractPayload({
			contract: CONTRACT,
			config: { id: "demo", name: "Demo", description: "" },
			warnings: [],
		});
		expect(bare.fixtures).toEqual({});
	});
});

describe("resolveWidgetConfigPath", () => {
	const groups = [{ name: "react", dir: "/proj/widgets/react" }];

	test("resolves known group + plain widget id", () => {
		expect(resolveWidgetConfigPath(groups, "react", "hello-widget")).toBe(
			join(
				"/proj/widgets/react",
				"src",
				"widgets",
				"hello-widget",
				"widget.config.ts",
			),
		);
	});

	test("rejects unknown groups and non-kebab ids (path traversal)", () => {
		expect(resolveWidgetConfigPath(groups, "vue", "hello-widget")).toBeNull();
		expect(resolveWidgetConfigPath(groups, "react", "../evil")).toBeNull();
		expect(resolveWidgetConfigPath(groups, "react", "..")).toBeNull();
		expect(resolveWidgetConfigPath(groups, "react", "Evil")).toBeNull();
		expect(resolveWidgetConfigPath(groups, "react", "a/b")).toBeNull();
	});
});

describe("handleContractRequest", () => {
	test("404s for unknown widgets without extracting", () => {
		const response = handleContractRequest(
			new ContractCache(),
			[{ name: "react", dir: "/nonexistent" }],
			"react",
			"missing",
		);
		expect(response.status).toBe(404);
		expect(response.body).toEqual({ error: "Unknown widget 'react/missing'" });
	});

	test("serves the extracted contract with fixtures, caches by mtime, and picks up edits", () => {
		const fixture = makeProjectFixture();
		const fixturedConfig = HELLO_WIDGET_CONFIG.replace(
			"sizing: { defaultHeight: 200, resizable: false, maxHeight: 600 },",
			'sizing: { defaultHeight: 200, resizable: false, maxHeight: 600 },\n\tdev: { fixtures: { loud: { greeting: "HELLO!" } } },',
		);
		writeFileSync(fixture.widgetConfigPath, fixturedConfig);

		const cache = new ContractCache();
		const groups = [{ name: "react", dir: fixture.groupDir }];
		const response = handleContractRequest(
			cache,
			groups,
			"react",
			"hello-widget",
		);
		expect(response.status).toBe(200);
		if ("error" in response.body) throw new Error(response.body.error);
		expect(response.body.contract.id).toBe("hello-widget");
		expect(response.body.contract.inputs.greeting?.default).toBe("Hello");
		expect(response.body.formModel).toEqual([
			{
				key: "greeting",
				label: "greeting",
				description: "Greeting text",
				optional: false,
				default: "Hello",
				control: { kind: "text" },
			},
		]);
		expect(response.body.fixtures).toEqual({
			loud: { greeting: "HELLO!" },
		});

		// same mtime → cached extraction result (identity)
		expect(cache.get(fixture.widgetConfigPath)).toBe(
			cache.get(fixture.widgetConfigPath),
		);

		// config edit with a new mtime → re-extracted on the next request
		writeFileSync(
			fixture.widgetConfigPath,
			fixturedConfig.replace('@default "Hello"', '@default "Hi"'),
		);
		const bumped = new Date(Date.now() + 5000);
		utimesSync(fixture.widgetConfigPath, bumped, bumped);
		const updated = handleContractRequest(
			cache,
			groups,
			"react",
			"hello-widget",
		);
		expect(updated.status).toBe(200);
		if ("error" in updated.body) throw new Error(updated.body.error);
		expect(updated.body.contract.inputs.greeting?.default).toBe("Hi");
	}, 120000);
});

describe("harnessHtml", () => {
	const widgets: HarnessWidget[] = [
		{
			group: "react",
			id: "hello-widget",
			entryUrl: widgetEntryUrl(4701, "hello-widget"),
		},
		{
			group: "svelte",
			id: "kpi-card",
			entryUrl: widgetEntryUrl(4702, "kpi-card"),
		},
	];
	const html = harnessHtml(widgets);

	test("lists every widget as group + id", () => {
		expect(html).toContain('data-key="react/hello-widget"');
		expect(html).toContain('data-key="svelte/kpi-card"');
		expect(html).toContain('<div class="group-name">react</div>');
		expect(html).toContain('<div class="group-name">svelte</div>');
	});

	test("embeds the harness data with entry URLs, protocol, and theme tokens", () => {
		expect(html).toContain("window.__HARNESS__ = ");
		expect(html).toContain(
			"http://localhost:4701/src/widgets/hello-widget/index.html",
		);
		expect(html).toContain('"protocol":"flw/1"');
		expect(html).toContain('"--background"');
	});

	test("mounts widgets in a sandboxed iframe", () => {
		expect(html).toContain('setAttribute("sandbox", "allow-scripts")');
	});

	test("is fully self-contained (no external scripts, styles, or hosts)", () => {
		expect(html).not.toMatch(/<script[^>]+src=/i);
		expect(html).not.toMatch(/<link\b/i);
		expect(html).not.toMatch(/https?:\/\/(?!localhost)/);
	});

	test("escapes markup-breaking characters in widget metadata", () => {
		const hostile = harnessHtml([
			{
				group: "<evil>",
				id: "x</script>",
				entryUrl: "http://localhost:4701/x",
			},
		]);
		expect(hostile).not.toContain("<evil>");
		expect(hostile).not.toContain("x</script>");
		expect(hostile).toContain("&lt;evil&gt;");
		expect(hostile).toContain("\\u003c/script>");
	});
});

describe("widgetEntryUrl", () => {
	test("composes the child dev server document URL", () => {
		expect(widgetEntryUrl(4711, "sales-chart")).toBe(
			"http://localhost:4711/src/widgets/sales-chart/index.html",
		);
	});
});
