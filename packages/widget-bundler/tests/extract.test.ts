import { describe, expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { contractToJson } from "../src/contract-types";
import { extractContract } from "../src/extract";
import { tmpDir } from "./helpers";

const FIXTURE = join(
	import.meta.dir,
	"fixtures",
	"sales-chart",
	"widget.config.ts",
);

const SALES_ROW_SCHEMA = {
	type: "object",
	properties: { x: { type: "string" }, y: { type: "number" } },
	required: ["x", "y"],
};

function writeTmpWidget(content: string): string {
	const dir = tmpDir("flwb-extract");
	mkdirSync(dir, { recursive: true });
	const path = join(dir, "widget.config.ts");
	writeFileSync(path, content);
	return path;
}

describe("extractContract", () => {
	test("derives the exact contract from the design doc example", () => {
		const { contract, config, warnings } = extractContract(FIXTURE);

		expect(contract).toEqual({
			contractVersion: 1,
			id: "sales-chart",
			inputs: {
				title: {
					type: "string",
					description: "Chart headline",
					default: "Sales",
				},
				variant: {
					type: "enum",
					choices: ["bar", "line"],
					description: "Chart style",
					default: "bar",
				},
				limit: {
					type: "number",
					description: "Max points",
					default: 50,
					min: 1,
					max: 500,
				},
				rows: {
					type: "json",
					schema: { type: "array", items: SALES_ROW_SCHEMA },
				},
				showLegend: {
					type: "boolean",
					description: "Show the legend",
					optional: true,
				},
			},
			events: {
				pointSelected: {
					payloadSchema: SALES_ROW_SCHEMA,
					description: "Fired when a data point is clicked",
				},
				refreshRequested: { payloadSchema: null },
			},
			queries: {
				getSelection: {
					argsSchema: null,
					resultSchema: {
						type: "object",
						properties: {
							rows: { type: "array", items: SALES_ROW_SCHEMA },
						},
						required: ["rows"],
					},
				},
				getValue: { argsSchema: null, resultSchema: { type: "string" } },
			},
			sizing: { defaultHeight: 320, resizable: true },
		});

		expect(config).toEqual({
			id: "sales-chart",
			name: "Sales Chart",
			description: "Interactive bar/line chart",
			sizing: { defaultHeight: 320, resizable: true },
			fixtures: {
				empty: { rows: [] },
				loaded: { title: "Q3 Sales" },
			},
		});

		expect(warnings).toHaveLength(1);
		expect(warnings[0]).toContain("'rows'");
		expect(warnings[0]).toContain("@default");
	}, 30000);

	test("canonical JSON matches serde field order and skip semantics", () => {
		const { contract } = extractContract(FIXTURE);
		const json = contractToJson(contract);
		const parsed = JSON.parse(json);

		expect(Object.keys(parsed)).toEqual([
			"contractVersion",
			"id",
			"inputs",
			"events",
			"queries",
			"sizing",
		]);
		expect(Object.keys(parsed.inputs)).toEqual([
			"limit",
			"rows",
			"showLegend",
			"title",
			"variant",
		]);
		expect(Object.keys(parsed.inputs.title)).toEqual([
			"type",
			"description",
			"default",
		]);
		expect(json).not.toContain('"optional": false');
		expect(parsed.events.refreshRequested).toEqual({ payloadSchema: null });
		expect(json).not.toContain("$ref");
	}, 30000);

	test("rejects non-empty inline type literals", () => {
		const path =
			writeTmpWidget(`import { defineWidget } from "@flow-like/widget-sdk";

export default defineWidget<{ title: string }, {}, {}>({
	id: "inline-widget",
	name: "Inline",
});
`);
		expect(() => extractContract(path)).toThrow(/inline type literal/);
	}, 30000);

	test("allows empty inline literals as empty sections", () => {
		const path =
			writeTmpWidget(`import { defineWidget } from "@flow-like/widget-sdk";

interface Inputs {
	/** @default true */
	enabled: boolean;
}

export default defineWidget<Inputs, {}, {}>({
	id: "tiny-widget",
	name: "Tiny",
});
`);
		const { contract } = extractContract(path);
		expect(contract.inputs.enabled).toEqual({
			type: "boolean",
			default: true,
		});
		expect(contract.events).toEqual({});
		expect(contract.queries).toEqual({});
		expect(contract.sizing).toEqual({ defaultHeight: 320, resizable: true });
	}, 30000);

	test("fails on recursive types with a clear error", () => {
		const path =
			writeTmpWidget(`import { defineWidget } from "@flow-like/widget-sdk";

interface TreeNode {
	value: string;
	children: TreeNode[];
}

interface Inputs {
	tree: TreeNode;
}

export default defineWidget<Inputs, {}, {}>({
	id: "tree-widget",
	name: "Tree",
});
`);
		expect(() => extractContract(path)).toThrow(/Recursive type/);
		expect(() => extractContract(path)).toThrow(/TreeNode/);
	}, 30000);

	test("fails on computed config properties, naming the property", () => {
		const path =
			writeTmpWidget(`import { defineWidget } from "@flow-like/widget-sdk";

const label = "Computed";

export default defineWidget<{}, {}, {}>({
	id: "computed-widget",
	name: label,
});
`);
		expect(() => extractContract(path)).toThrow(/'name'/);
		expect(() => extractContract(path)).toThrow(/literal/);
	}, 30000);

	test("rejects invalid widget ids", () => {
		const path =
			writeTmpWidget(`import { defineWidget } from "@flow-like/widget-sdk";

export default defineWidget<{}, {}, {}>({
	id: "Bad_Id",
	name: "Bad",
});
`);
		expect(() => extractContract(path)).toThrow(/Invalid widget id/);
	}, 30000);
});
