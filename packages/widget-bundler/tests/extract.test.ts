import { describe, expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
	BASE_CONTRACT_VERSION,
	CONTRACT_VERSION,
	type WidgetContract,
	canonicalizeContract,
	contractToJson,
	validateContract,
} from "../src/contract-types";
import { extractContract } from "../src/extract";
import { HELLO_WIDGET_CONFIG, tmpDir } from "./helpers";

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

	test("extracts @mutation on query members and omits false defaults", () => {
		const path =
			writeTmpWidget(`import { defineWidget } from "@flow-like/widget-sdk";

interface Queries {
	/** Change the map source.
	 * @mutation
	 */
	setMapSource: { args: { sourceId: string }; returns: { accepted: boolean } };
	/** Read the current map source. */
	getMapSource: { args: {}; returns: { sourceId: string } };
}

export default defineWidget<{}, {}, Queries>({
	id: "map-widget",
	name: "Map Widget",
});
	`);
		const { contract } = extractContract(path);
		expect(contract.queries.setMapSource?.mutation).toBeTrue();
		expect(contract.queries.setMapSource?.description).toBe(
			"Change the map source.",
		);
		expect(contract.queries.getMapSource?.mutation).toBeUndefined();

		const serialized = JSON.parse(contractToJson(contract));
		expect(serialized.queries.setMapSource.mutation).toBeTrue();
		expect(serialized.queries.getMapSource).not.toHaveProperty("mutation");
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

const RUST_CSP_CONTRACT_JSON = `{
  "contractVersion": 2,
  "id": "hello-widget",
  "inputs": {
    "greeting": {
      "type": "string",
      "description": "Greeting text",
      "default": "Hello"
    }
  },
  "events": {
    "dismissed": {
      "payloadSchema": null
    }
  },
  "queries": {
    "getGreeting": {
      "argsSchema": null,
      "resultSchema": {
        "type": "string"
      }
    }
  },
  "sizing": {
    "defaultHeight": 200,
    "resizable": false,
    "maxHeight": 600
  },
  "capabilities": {
    "workers": true
  },
  "csp": {
    "connectSrc": [
      "https://api.maptiler.com",
      "https://xn--bcher-kva.de",
      "wss://live.example.com"
    ],
    "imgSrc": [
      "https://a.tile.openstreetmap.org",
      "https://b.tile.openstreetmap.org"
    ],
    "fontSrc": [
      "https://fonts.gstatic.com"
    ],
    "styleSrc": [
      "https://fonts.googleapis.com"
    ]
  }
}`;

const RUST_PLAIN_CONTRACT_JSON = `{
  "contractVersion": 1,
  "id": "plain-widget",
  "inputs": {},
  "events": {},
  "queries": {},
  "sizing": {
    "defaultHeight": 320,
    "resizable": true
  },
  "capabilities": {
    "media": false,
    "wasm": true
  }
}`;

function writeCspWidget(declaration: string): string {
	return writeTmpWidget(
		HELLO_WIDGET_CONFIG.replace(
			'id: "hello-widget",',
			`id: "hello-widget",\n\t${declaration},`,
		),
	);
}

describe("csp declarations", () => {
	test("normalize into a v2 contract whose bytes match the Rust WidgetBundleBuilder", () => {
		const path = writeCspWidget(`capabilities: { workers: true },
	csp: {
		styleSrc: ["https://fonts.googleapis.com"],
		mediaSrc: [],
		imgSrc: [
			"https://B.tile.openstreetmap.org",
			"https://a.tile.openstreetmap.org",
			"https://b.tile.openstreetmap.org",
		],
		fontSrc: ['https://fonts.gstatic.com'],
		connectSrc: ["wss://live.example.com", "HTTPS://API.maptiler.com", "https://bücher.de"] as const,
	}`);
		const { contract, config } = extractContract(path);
		expect(contract.contractVersion).toBe(CONTRACT_VERSION);
		expect(config.csp).toEqual(contract.csp);
		expect(validateContract(contract)).toEqual([]);
		expect(contractToJson(contract)).toBe(RUST_CSP_CONTRACT_JSON);
	}, 30000);

	test("an empty declaration stays a v1 contract without csp", () => {
		const path = writeCspWidget("csp: { connectSrc: [], imgSrc: [] }");
		const { contract, config } = extractContract(path);
		expect(contract.contractVersion).toBe(BASE_CONTRACT_VERSION);
		expect(contract).not.toHaveProperty("csp");
		expect(config).not.toHaveProperty("csp");
		expect(JSON.parse(contractToJson(contract))).not.toHaveProperty("csp");
	}, 30000);

	test("rejects directives outside the extendable set", () => {
		for (const directive of ["scriptSrc", "frameSrc", "workerSrc"]) {
			const path = writeCspWidget(
				`csp: { ${directive}: ["https://cdn.example.org"] }`,
			);
			expect(() => extractContract(path)).toThrow(
				`Invalid widget csp directive "${directive}" for widget hello-widget`,
			);
		}
	}, 60000);

	test("names the widget, directive, source and reason for rejected sources", () => {
		const cases: [string, string][] = [
			[
				'csp: { connectSrc: ["https://*.maptiler.com"] }',
				'Invalid widget csp source "https://*.maptiler.com" in connectSrc for widget hello-widget: wildcards are not allowed',
			],
			[
				'csp: { imgSrc: ["https://tiles.example.org:443"] }',
				'Invalid widget csp source "https://tiles.example.org:443" in imgSrc for widget hello-widget: ports are not allowed',
			],
			[
				'csp: { connectSrc: ["http://api.example.org"] }',
				'Invalid widget csp source "http://api.example.org" in connectSrc for widget hello-widget: scheme is not allowed for this directive',
			],
			[
				"csp: { fontSrc: [\"'self'\"] }",
				"Invalid widget csp source \"'self'\" in fontSrc for widget hello-widget: keywords, nonces and hashes are not allowed",
			],
			[
				"csp: { mediaSrc: [42] }",
				"Invalid widget csp source 42 in mediaSrc for widget hello-widget: must be a string",
			],
		];
		for (const [declaration, message] of cases) {
			const path = writeCspWidget(declaration);
			expect(() => extractContract(path)).toThrow(message);
		}
	}, 90000);

	test("rejects non-literal and malformed declarations", () => {
		const computed = writeTmpWidget(
			`const HOST = "https://api.example.org";\n${HELLO_WIDGET_CONFIG.replace(
				'id: "hello-widget",',
				'id: "hello-widget", csp: { connectSrc: [HOST] },',
			)}`,
		);
		expect(() => extractContract(computed)).toThrow(
			"'csp.connectSrc[0]' must be a literal",
		);
		expect(() =>
			extractContract(writeCspWidget('csp: ["https://api.example.org"]')),
		).toThrow("Widget csp for widget hello-widget");
		expect(() =>
			extractContract(
				writeCspWidget('csp: { connectSrc: "https://api.example.org" }'),
			),
		).toThrow("must be an array of string literals");
	}, 90000);

	test("caps sources after deduplication", () => {
		const hosts = (count: number) =>
			Array.from(
				{ length: count },
				(_, index) => `"https://h${index}.example.org"`,
			).join(", ");
		const atCap = writeCspWidget(
			`csp: { connectSrc: [${hosts(16)}, "https://H0.example.org"] }`,
		);
		expect(extractContract(atCap).contract.csp?.connectSrc).toHaveLength(16);
		const overCap = writeCspWidget(
			`csp: { connectSrc: [${hosts(12)}], imgSrc: [${hosts(5)}] }`,
		);
		expect(() => extractContract(overCap)).toThrow(
			"Widget csp for widget hello-widget declares 17 sources; at most 16 are allowed",
		);
	}, 60000);
});

describe("contract version rule", () => {
	const base: WidgetContract = {
		contractVersion: BASE_CONTRACT_VERSION,
		id: "live-map",
		inputs: {},
		events: {},
		queries: {},
		sizing: { defaultHeight: 320, resizable: true },
	};
	const csp = { connectSrc: ["https://api.maptiler.com"] };

	test("v1 without csp and v2 with csp are valid", () => {
		expect(validateContract(base)).toEqual([]);
		expect(
			validateContract({ ...base, contractVersion: CONTRACT_VERSION, csp }),
		).toEqual([]);
	});

	test("mirrors the Rust errors in both directions", () => {
		expect(validateContract({ ...base, csp })).toEqual([
			"Widget 'live-map' declares csp and must use contractVersion 2",
		]);
		expect(
			validateContract({ ...base, contractVersion: CONTRACT_VERSION }),
		).toEqual([
			"Widget 'live-map' uses contractVersion 2 without csp; contracts without csp must use contractVersion 1",
		]);
		for (const contractVersion of [0, 3, 99]) {
			expect(validateContract({ ...base, contractVersion, csp })).toEqual([
				`Unsupported contractVersion ${contractVersion} for widget 'live-map' (supported: 1, or 2 with csp)`,
			]);
		}
	});

	test("rejects empty, unknown and non-canonical csp", () => {
		expect(
			validateContract({ ...base, contractVersion: CONTRACT_VERSION, csp: {} }),
		).toEqual([
			"Widget 'live-map' declares an empty csp; omit csp when it grants no sources",
		]);
		expect(
			validateContract({
				...base,
				contractVersion: CONTRACT_VERSION,
				csp: {
					connectSrc: ["https://b.example.org", "http://a.example.org"],
				},
			}),
		).toEqual([
			"Widget 'live-map': Invalid csp source \"http://a.example.org\" in connectSrc: scheme is not allowed for this directive",
			"Widget 'live-map': csp connectSrc must be sorted ascending without duplicates",
		]);
		const parsed = JSON.parse(
			'{"contractVersion":2,"id":"live-map","inputs":{},"events":{},"queries":{},"csp":{"scriptSrc":["https://cdn.example.org"]}}',
		) as WidgetContract;
		expect(validateContract(parsed)).toContain(
			"Widget 'live-map': csp declares unknown directive \"scriptSrc\" (allowed: connectSrc, imgSrc, fontSrc, mediaSrc, styleSrc)",
		);
	});

	test("canonicalization derives the version from csp and matches Rust bytes", () => {
		const plain: WidgetContract = {
			...base,
			id: "plain-widget",
			contractVersion: CONTRACT_VERSION,
			capabilities: { wasm: true, media: false },
			csp: { connectSrc: [], imgSrc: [] },
		};
		expect(contractToJson(plain)).toBe(RUST_PLAIN_CONTRACT_JSON);

		const canonical = canonicalizeContract({
			...base,
			csp: {
				styleSrc: ["https://fonts.googleapis.com"],
				connectSrc: [
					"wss://live.example.com",
					"https://api.maptiler.com",
					"wss://live.example.com",
				],
			},
		});
		expect(canonical.contractVersion).toBe(CONTRACT_VERSION);
		expect(Object.keys(canonical)).toEqual([
			"contractVersion",
			"id",
			"inputs",
			"events",
			"queries",
			"sizing",
			"csp",
		]);
		expect(canonical.csp).toEqual({
			connectSrc: ["https://api.maptiler.com", "wss://live.example.com"],
			styleSrc: ["https://fonts.googleapis.com"],
		});
		expect(validateContract(canonical)).toEqual([]);
	});
});
