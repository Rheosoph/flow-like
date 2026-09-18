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
import { extractContract, networkInputSchemaErrors } from "../src/extract";
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
  "csp": [
    {
      "reason": "Loads map tiles and live positions",
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
    },
    {
      "reason": "Loads map layers from Amazon S3 buckets in Frankfurt",
      "connectSrc": [
        "https://*.s3.eu-central-1.amazonaws.com"
      ]
    },
    {
      "reason": "Loads a greeting image given to it at runtime",
      "inputs": [
        {
          "path": "greeting",
          "directives": [
            "connectSrc",
            "imgSrc"
          ]
        }
      ]
    }
  ]
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

const REASON = 'reason: "Loads map tiles"';

describe("csp declarations", () => {
	test("normalize into a v2 contract whose bytes match the Rust WidgetBundleBuilder", () => {
		const path = writeCspWidget(`capabilities: { workers: true },
	csp: [
		{
			reason: "  Loads map   tiles and\\nlive positions ",
			styleSrc: ["https://fonts.googleapis.com"],
			mediaSrc: [],
			imgSrc: [
				"https://B.tile.openstreetmap.org",
				"https://a.tile.openstreetmap.org",
				"https://b.tile.openstreetmap.org",
			],
			fontSrc: ['https://fonts.gstatic.com'],
			connectSrc: ["wss://live.example.com", "HTTPS://API.maptiler.com", "https://bücher.de"] as const,
		},
		{
			reason: \`Loads map layers from Amazon S3 buckets in Frankfurt\`,
			connectSrc: ["https://*.S3.eu-central-1.amazonaws.com"],
		},
		{
			reason: "Loads a greeting image given to it at runtime",
			inputs: [{ path: "greeting", directives: ["imgSrc", "connectSrc", "imgSrc"] }],
		},
	]`);
		const { contract, config, warnings } = extractContract(path);
		expect(contract.contractVersion).toBe(CONTRACT_VERSION);
		expect(config.csp).toEqual(contract.csp);
		expect(validateContract(contract)).toEqual([]);
		expect(warnings).toEqual([]);
		expect(contractToJson(contract)).toBe(RUST_CSP_CONTRACT_JSON);
	}, 30000);

	test("an empty declaration stays a v1 contract without csp", () => {
		const path = writeCspWidget("csp: []");
		const { contract, config } = extractContract(path);
		expect(contract.contractVersion).toBe(BASE_CONTRACT_VERSION);
		expect(contract).not.toHaveProperty("csp");
		expect(config).not.toHaveProperty("csp");
		expect(JSON.parse(contractToJson(contract))).not.toHaveProperty("csp");
	}, 30000);

	test("rejects keys outside the purpose shape", () => {
		for (const key of ["scriptSrc", "frameSrc", "workerSrc", "level"]) {
			const path = writeCspWidget(
				`csp: [{ ${REASON}, ${key}: ["https://cdn.example.org"] }]`,
			);
			expect(() => extractContract(path)).toThrow(
				`Invalid widget csp key "${key}" in csp[0] for widget hello-widget`,
			);
		}
	}, 60000);

	test("names the widget, purpose, key and value of rejected entries", () => {
		const cases: [string, string][] = [
			[
				`csp: [{ ${REASON}, connectSrc: ["https://a.*.maptiler.com"] }]`,
				'Invalid widget csp source "https://a.*.maptiler.com" in csp[0].connectSrc for widget hello-widget: wildcards are only allowed as a leading "*." label of the host',
			],
			[
				`csp: [{ ${REASON}, imgSrc: ["https://*.co.uk"] }]`,
				'Invalid widget csp source "https://*.co.uk" in csp[0].imgSrc for widget hello-widget: wildcard base is a public suffix or spans public suffixes',
			],
			[
				`csp: [{ ${REASON}, imgSrc: ["https://*.sslip.io"] }]`,
				'Invalid widget csp source "https://*.sslip.io" in csp[0].imgSrc for widget hello-widget: host uses a reserved or local-only name',
			],
			[
				`csp: [{ ${REASON}, imgSrc: ["https://tiles.example.org:443"] }]`,
				'Invalid widget csp source "https://tiles.example.org:443" in csp[0].imgSrc for widget hello-widget: ports are not allowed',
			],
			[
				`csp: [{ ${REASON}, connectSrc: ["http://api.example.org"] }]`,
				'Invalid widget csp source "http://api.example.org" in csp[0].connectSrc for widget hello-widget: scheme is not allowed for this directive',
			],
			[
				`csp: [{ ${REASON}, fontSrc: ["'self'"] }]`,
				"Invalid widget csp source \"'self'\" in csp[0].fontSrc for widget hello-widget: keywords, nonces and hashes are not allowed",
			],
			[
				`csp: [{ ${REASON}, mediaSrc: [42] }]`,
				"Invalid widget csp source 42 in csp[0].mediaSrc for widget hello-widget: must be a string",
			],
			[
				'csp: [{ reason: "Loads tiles from cesium.com", imgSrc: ["https://a.example.org"] }]',
				'Invalid widget csp reason "Loads tiles from cesium.com" in csp[0].reason for widget hello-widget: reason must not contain web addresses, email addresses or domain names (reason-contains-address)',
			],
			[
				'csp: [{ reason: "Securely loads map tiles", imgSrc: ["https://a.example.org"] }]',
				'Invalid widget csp reason "Securely loads map tiles" in csp[0].reason for widget hello-widget: reason must not claim that sources are verified, trusted, approved, official, certified, secure or safe (reason-claims-assurance)',
			],
			[
				`csp: [{ ${REASON}, imgSrc: ["https://a.example.org"] }, { reason: "LOADS  MAP TILES", imgSrc: ["https://b.example.org"] }]`,
				'Invalid widget csp reason "LOADS MAP TILES" in csp[1].reason for widget hello-widget: reasons of two purposes must differ (reason-duplicate); csp[0].reason reads the same',
			],
			[
				`csp: [{ ${REASON}, inputs: [{ path: "greeting", directives: ["frameSrc"] }] }]`,
				'Invalid widget csp directive "frameSrc" in csp[0].inputs[0].directives for widget hello-widget: only connectSrc, imgSrc, fontSrc, mediaSrc, styleSrc can be extended',
			],
			[
				`csp: [{ ${REASON}, inputs: [{ path: "greeting", directives: ["imgSrc"], template: { wildcard: true } }] }]`,
				'Invalid widget csp key "wildcard" in csp[0].inputs[0].template for widget hello-widget: allowed keys are subdomains, subdomainsInput',
			],
		];
		for (const [declaration, message] of cases) {
			const path = writeCspWidget(declaration);
			expect(() => extractContract(path)).toThrow(message);
		}
	}, 120000);

	test("rejects non-literal and malformed declarations", () => {
		const computed = writeTmpWidget(
			`const HOST = "https://api.example.org";\n${HELLO_WIDGET_CONFIG.replace(
				'id: "hello-widget",',
				`id: "hello-widget", csp: [{ ${REASON}, connectSrc: [HOST] }],`,
			)}`,
		);
		expect(() => extractContract(computed)).toThrow(
			"'csp[0].connectSrc[0]' must be a literal",
		);
		expect(() =>
			extractContract(
				writeCspWidget('csp: { connectSrc: ["https://api.example.org"] }'),
			),
		).toThrow("must be an array of purpose groups");
		expect(() =>
			extractContract(
				writeCspWidget(
					`csp: [{ ${REASON}, connectSrc: "https://api.example.org" }]`,
				),
			),
		).toThrow(
			"Invalid widget csp csp[0].connectSrc for widget hello-widget: must be an array of string literals",
		);
		expect(() =>
			extractContract(
				writeCspWidget('csp: [{ connectSrc: ["https://api.example.org"] }]'),
			),
		).toThrow(
			"Invalid widget csp csp[0] for widget hello-widget: must be an object with a string 'reason'",
		);
	}, 90000);

	test("caps sources across purposes after deduplication", () => {
		const hosts = (from: number, count: number) =>
			Array.from(
				{ length: count },
				(_, index) => `"https://h${from + index}.example.org"`,
			).join(", ");
		const atCap = writeCspWidget(
			`csp: [{ ${REASON}, connectSrc: [${hosts(0, 8)}, "https://H0.example.org"] }, { reason: "Loads map labels", imgSrc: [${hosts(8, 8)}] }]`,
		);
		expect(extractContract(atCap).contract.csp?.[0]?.connectSrc).toHaveLength(
			8,
		);
		const overCap = writeCspWidget(
			`csp: [{ ${REASON}, connectSrc: [${hosts(0, 12)}] }, { reason: "Loads map labels", imgSrc: [${hosts(12, 5)}] }]`,
		);
		expect(() => extractContract(overCap)).toThrow(
			"Widget 'hello-widget': csp declares 17 sources; at most 16 are allowed",
		);
	}, 60000);
});

const SLOT_WIDGET = `import { defineWidget } from "@flow-like/widget-sdk";

interface Layer {
	url: string;
	zoom: number;
}

interface Inputs {
	/** @default "https://{s}.tiles.customer-maps.com/{z}/{x}/{y}.png" */
	tileUrl: string;
	/** @default [{ "url": "https://acme.s3.eu-central-1.amazonaws.com/a.png", "zoom": 1 }] */
	layers: Layer[];
	sources?: Record<string, { href: string }>;
	/** @default "https://media.example.org/live.m3u8" */
	videoUrl: string;
}

export default defineWidget<Inputs, {}, {}>({
	id: "slot-widget",
	name: "Slot widget",
	description: "Network inputs",
	csp: [CSP],
});
`;

function writeSlotWidget(csp: string): string {
	return writeTmpWidget(SLOT_WIDGET.replace("[CSP]", csp));
}

describe("network input slots", () => {
	test("paths must reach a string through the generated schema", () => {
		const ok = writeSlotWidget(`[
		{
			reason: "Loads map layers given to it at runtime",
			imgSrc: ["https://*.s3.eu-central-1.amazonaws.com", "https://*.tiles.customer-maps.com"],
			inputs: [
				{ path: "layers[].url", directives: ["imgSrc"] },
				{ path: "sources.*.href", directives: ["imgSrc"] },
				{ path: "tileUrl", directives: ["imgSrc"], template: { subdomains: ["a", "b"] } },
			],
		},
	]`);
		const { contract, warnings } = extractContract(ok);
		expect(networkInputSchemaErrors(contract)).toEqual([]);
		expect(warnings).toEqual([]);

		for (const [path, root] of [
			["layers[].zoom", "layers"],
			["layers.*.url", "layers"],
			["sources[].href", "sources"],
		] as const) {
			const broken = writeSlotWidget(`[
		{
			reason: "Loads map layers given to it at runtime",
			inputs: [{ path: "${path}", directives: ["imgSrc"] }],
		},
	]`);
			expect(() => extractContract(broken)).toThrow(
				`Widget 'slot-widget': csp purpose 0: input "${path}" does not reach a string through the schema of input "${root}"`,
			);
		}
	}, 90000);

	test("warns about mediaSrc without connectSrc and undeclared default hosts", () => {
		const path = writeSlotWidget(`[
		{
			reason: "Plays the live video stream",
			mediaSrc: ["https://media.example.org"],
		},
		{
			reason: "Loads map tiles given to it at runtime",
			inputs: [
				{ path: "videoUrl", directives: ["mediaSrc"] },
				{ path: "tileUrl", directives: ["imgSrc", "connectSrc"], template: { subdomains: ["a", "b"] } },
			],
		},
	]`);
		const { warnings } = extractContract(path);
		expect(warnings).toEqual([
			"Widget 'slot-widget': csp purpose 1: input \"tileUrl\" has a @default URL on https://a.tiles.customer-maps.com that no static connectSrc, imgSrc source declares; the SDK merges defaults inside the widget, so the host never sees or approves them. Declare the origin statically or send the URL as an input value",
			"Widget 'slot-widget': csp purpose 1: input \"tileUrl\" has a @default URL on https://b.tiles.customer-maps.com that no static connectSrc, imgSrc source declares; the SDK merges defaults inside the widget, so the host never sees or approves them. Declare the origin statically or send the URL as an input value",
			"Widget 'slot-widget': csp purpose 1: input \"videoUrl\" feeds mediaSrc without connectSrc; hls.js and MSE players fetch through connectSrc, native HLS uses mediaSrc",
		]);
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
	const csp = [
		{ reason: "Loads map tiles", connectSrc: ["https://api.maptiler.com"] },
	];

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
			validateContract({ ...base, contractVersion: CONTRACT_VERSION, csp: [] }),
		).toEqual([
			"Widget 'live-map' declares an empty csp; omit csp when it declares no purposes",
		]);
		expect(
			validateContract({
				...base,
				contractVersion: CONTRACT_VERSION,
				csp: [
					{
						reason: "Loads map tiles",
						connectSrc: ["https://b.example.org", "http://a.example.org"],
					},
				],
			}),
		).toEqual([
			"Widget 'live-map': csp purpose 0: Invalid csp source \"http://a.example.org\" in connectSrc: scheme is not allowed for this directive",
			"Widget 'live-map': csp purpose 0: connectSrc must be sorted ascending without duplicates",
		]);
		const parsed = JSON.parse(
			'{"contractVersion":2,"id":"live-map","inputs":{},"events":{},"queries":{},"csp":[{"reason":"Loads map tiles","scriptSrc":["https://cdn.example.org"]}]}',
		) as WidgetContract;
		expect(validateContract(parsed)).toEqual([
			"Widget 'live-map': csp purpose 0: declares unknown key \"scriptSrc\" (allowed: reason, connectSrc, imgSrc, fontSrc, mediaSrc, styleSrc, inputs)",
		]);
	});

	test("canonicalization derives the version from csp and matches Rust bytes", () => {
		const plain: WidgetContract = {
			...base,
			id: "plain-widget",
			contractVersion: CONTRACT_VERSION,
			capabilities: { wasm: true, media: false },
			csp: [],
		};
		expect(contractToJson(plain)).toBe(RUST_PLAIN_CONTRACT_JSON);

		const canonical = canonicalizeContract({
			...base,
			csp: [
				{
					reason: "Loads map tiles",
					styleSrc: ["https://fonts.googleapis.com"],
					connectSrc: [
						"wss://live.example.com",
						"https://api.maptiler.com",
						"wss://live.example.com",
					],
				},
			],
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
		expect(canonical.csp).toEqual([
			{
				reason: "Loads map tiles",
				connectSrc: ["https://api.maptiler.com", "wss://live.example.com"],
				styleSrc: ["https://fonts.googleapis.com"],
			},
		]);
		expect(validateContract(canonical)).toEqual([]);
	});
});
