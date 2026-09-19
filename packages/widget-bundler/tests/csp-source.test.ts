import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import type { WidgetCspPurpose } from "@flow-like/widget-sdk";
import {
	type WidgetContract,
	canonicalizeContract,
	contractToJson,
	normalizeCspPurposes,
	validateContract,
} from "../src/contract-types";
import {
	CSP_SOURCE_REJECTION_MESSAGES,
	type CspDirective,
	type CspSourceRejection,
	compareCspSources,
	flattenCspPurposes,
	isCspDirective,
	isValidWidgetInputPath,
	normalizeCspSource,
	parseWidgetInputPath,
	validateCspSource,
} from "../src/csp-source";

interface SourceCase {
	directive: string;
	source: string;
	reason?: string;
}

interface ContractCase {
	name: string;
	contract: Partial<WidgetContract>;
	error?: string;
}

interface GroupedCase {
	name: string;
	authored: Partial<WidgetContract>;
	canonical: WidgetContract;
	declaredCsp: Record<string, string[]>;
}

interface CspFixture {
	acceptedSources: SourceCase[];
	rejectedSources: SourceCase[];
	groupedContracts: GroupedCase[];
	invalidContracts: ContractCase[];
	unparsableContracts: ContractCase[];
}

const FIXTURE: CspFixture = JSON.parse(
	readFileSync(
		join(
			import.meta.dir,
			"..",
			"..",
			"wasm",
			"schema",
			"tests",
			"fixtures",
			"widget_csp.json",
		),
		"utf8",
	),
);

const CSP_SOURCE_URL = pathToFileURL(
	join(import.meta.dir, "..", "src", "csp-source.ts"),
).href;

/** Hosts Node's and Bun's `domainToASCII` used to resolve differently */
const HOST_PARSER_DEPENDENT_SOURCES = [
	"https://bücher.de\\tiles",
	"https://bücher.de\\evil.com",
	"https://bü\tcher.de",
	"https://bücher.de\nx",
	"https://bü\rcher.de",
	"https://bü%41cher.de",
	"https://bü_cher.de",
	"https://BÜ!cher.de",
	"https://bü\uff1a.de",
	"https://bü\uff0f.de",
	"https://bü\uff20.de",
	"https://bü\u2100.de",
	"https://bü\u3000.de",
	"https://bü.127.0.0.1",
	"https://ü.0x7f",
	"https://\uff10x7f.0.0.1",
];

function nodeStripsTypes(): boolean {
	try {
		const probe = Bun.spawnSync([
			"node",
			"-p",
			"!process.versions.bun && Boolean(process.features.typescript)",
		]);
		return probe.exitCode === 0 && probe.stdout.toString().trim() === "true";
	} catch {
		return false;
	}
}

function directiveOf(entry: SourceCase): CspDirective {
	if (!isCspDirective(entry.directive)) {
		throw new Error(`Unknown fixture directive ${entry.directive}`);
	}
	return entry.directive;
}

describe("shared fixture", () => {
	test("accepts every accepted source", () => {
		expect(FIXTURE.acceptedSources.length).toBeGreaterThanOrEqual(10);
		for (const entry of FIXTURE.acceptedSources) {
			expect([
				entry.source,
				validateCspSource(directiveOf(entry), entry.source),
			]).toEqual([entry.source, null]);
		}
	});

	test("rejects every rejected source with the listed reason", () => {
		expect(FIXTURE.rejectedSources.length).toBeGreaterThanOrEqual(40);
		for (const entry of FIXTURE.rejectedSources) {
			expect([
				entry.source,
				validateCspSource(directiveOf(entry), entry.source),
			]).toEqual([entry.source, entry.reason as CspSourceRejection]);
		}
	});

	test("every rejection code has a message", () => {
		for (const entry of FIXTURE.rejectedSources) {
			expect(
				CSP_SOURCE_REJECTION_MESSAGES[entry.reason as CspSourceRejection],
			).toBeString();
		}
	});

	test("accepted sources are fixed points of normalization", () => {
		for (const entry of FIXTURE.acceptedSources) {
			expect(normalizeCspSource(entry.source)).toBe(entry.source);
		}
	});
});

describe("normalizeCspSource", () => {
	test("lowercases the whole source", () => {
		expect(normalizeCspSource("HTTPS://API.MapTiler.com")).toBe(
			"https://api.maptiler.com",
		);
		expect(normalizeCspSource("WSS://Live.Example.com")).toBe(
			"wss://live.example.com",
		);
	});

	test("converts internationalized hosts to punycode", () => {
		expect(normalizeCspSource("https://bücher.de")).toBe(
			"https://xn--bcher-kva.de",
		);
		expect(normalizeCspSource("https://BÜCHER.de")).toBe(
			"https://xn--bcher-kva.de",
		);
		expect(normalizeCspSource("https://пример.рф")).toBe(
			"https://xn--e1afmkfd.xn--p1ai",
		);
	});

	test("leaves what the grammar rejects for the grammar to report", () => {
		for (const [source, reason] of [
			["https://bücher.de:443", "port"],
			["https://bücher.de/path", "path"],
			["https://user@bücher.de", "non-ascii"],
			["https://api.maptiler.com ", "non-ascii"],
			["https://*.bücher.de:443", "port"],
			["https://a.*.Example.com", "wildcard"],
			["'self'", "keyword"],
			["Https:api.maptiler.com", "missing-scheme"],
			["https://bü.localhost", "reserved-name"],
		] as const) {
			expect([
				source,
				validateCspSource("connectSrc", normalizeCspSource(source)),
			]).toEqual([source, reason]);
		}
	});

	test("keeps IDNA mappings of non-ASCII characters", () => {
		for (const source of [
			"https://bücher\u3002de",
			"https://bü\u00adcher.de",
			"https://bü\u200bcher.de",
			"https://\uff42ücher.de",
		]) {
			expect([source, normalizeCspSource(source)]).toEqual([
				source,
				"https://xn--bcher-kva.de",
			]);
		}
	});

	test("never lets the runtime's host parser repair an invalid host", () => {
		for (const source of HOST_PARSER_DEPENDENT_SOURCES) {
			const normalized = normalizeCspSource(source);
			expect([source, normalized]).toEqual([source, source.toLowerCase()]);
			expect([source, validateCspSource("connectSrc", normalized)]).toEqual([
				source,
				"non-ascii",
			]);
		}
	});

	test.skipIf(!nodeStripsTypes())("gives the same result under Node", () => {
		const cases = [
			...HOST_PARSER_DEPENDENT_SOURCES,
			...FIXTURE.acceptedSources.map((entry) => entry.source),
			...FIXTURE.rejectedSources.map((entry) => entry.source),
			"https://BÜCHER.de",
			"https://пример.рф",
			"https://bücher.de:443",
			"https://bücher\u3002de",
			"https://straße.de",
			"https://مثال.إختبار",
		];
		const result = Bun.spawnSync(
			[
				"node",
				"--input-type=module",
				"-e",
				`import { normalizeCspSource } from ${JSON.stringify(CSP_SOURCE_URL)};
process.stdout.write(JSON.stringify(JSON.parse(process.argv[1]).map(normalizeCspSource)));`,
				JSON.stringify(cases),
			],
			{ stderr: "pipe" },
		);
		expect(result.stderr.toString()).toBe("");
		expect(JSON.parse(result.stdout.toString())).toEqual(
			cases.map(normalizeCspSource),
		);
	});
});

describe("compareCspSources", () => {
	test("orders by UTF-8 bytes like Rust", () => {
		expect(compareCspSources("https://a.com", "https://b.com")).toBeLessThan(0);
		expect(compareCspSources("https://a.com", "https://a.co")).toBeGreaterThan(
			0,
		);
		expect(compareCspSources("wss://a.com", "https://z.com")).toBeGreaterThan(
			0,
		);
		expect(compareCspSources("￿", "\u{10000}")).toBeLessThan(0);
		expect("￿" < "\u{10000}").toBeFalse();
	});
});

function withDefaults(contract: Partial<WidgetContract>): WidgetContract {
	return {
		contractVersion: 1,
		id: "",
		inputs: {},
		events: {},
		queries: {},
		sizing: { defaultHeight: 320, resizable: true },
		...contract,
	};
}

describe("purpose groups", () => {
	test("canonicalize to the Rust bytes of every grouped fixture", () => {
		expect(FIXTURE.groupedContracts.length).toBeGreaterThanOrEqual(3);
		for (const entry of FIXTURE.groupedContracts) {
			const canonical = canonicalizeContract(withDefaults(entry.authored));
			expect([entry.name, contractToJson(canonical)]).toEqual([
				entry.name,
				JSON.stringify(entry.canonical, null, 2),
			]);
			expect([entry.name, validateContract(canonical)]).toEqual([
				entry.name,
				[],
			]);
			expect([entry.name, validateContract(entry.canonical)]).toEqual([
				entry.name,
				[],
			]);
			expect([entry.name, flattenCspPurposes(canonical.csp ?? [])]).toEqual([
				entry.name,
				entry.declaredCsp,
			]);
		}
	});

	test("report the listed error for every invalid fixture contract", () => {
		expect(FIXTURE.invalidContracts.length).toBeGreaterThanOrEqual(25);
		for (const entry of FIXTURE.invalidContracts) {
			const errors = validateContract(withDefaults(entry.contract));
			expect([
				entry.name,
				errors.some((error) => error.includes(entry.error ?? "")),
				errors.every((error) => error.startsWith("Widget 'live-map'")),
			]).toEqual([entry.name, true, true]);
		}
	});

	test("reject every shape serde refuses to parse", () => {
		for (const entry of FIXTURE.unparsableContracts) {
			const errors = validateContract(withDefaults(entry.contract));
			expect([entry.name, errors.length > 0]).toEqual([entry.name, true]);
			expect(
				errors.every((error) => error.startsWith("Widget 'live-map': csp ")),
			).toBeTrue();
		}
	});

	test("names the purpose and key of shape errors", () => {
		const contract = withDefaults({
			contractVersion: 2,
			id: "live-map",
			csp: [
				{
					reason: "Loads map tiles given at runtime",
					inputs: [{ path: "tileUrl", directives: ["frameSrc"] }],
				},
			] as unknown as WidgetCspPurpose[],
		});
		expect(validateContract(contract)).toEqual([
			"Widget 'live-map': csp purpose 0: input 0: declares unknown directive \"frameSrc\" (allowed: connectSrc, imgSrc, fontSrc, mediaSrc, styleSrc)",
		]);
	});

	test("normalization folds whitespace, lowercases sources and keeps group order", () => {
		const normalized = normalizeCspPurposes([
			{
				reason: "  Loads map\u00a0tiles\n from  Cafe\u0301 servers ",
				imgSrc: ["https://B.example.com", "https://a.example.com"],
				connectSrc: ["https://*.Bücher.de", "https://a.example.com"],
			},
			{
				reason: "Loads runtime tiles",
				inputs: [
					{ path: "z", directives: ["imgSrc", "connectSrc", "imgSrc"] },
					{
						path: "a",
						directives: ["mediaSrc"],
						template: { subdomains: ["b", "a", "b"] },
					},
					{ path: "m", directives: ["imgSrc"], template: {} },
				],
			},
		]);
		expect(normalized).toEqual([
			{
				reason: "Loads map tiles from Caf\u00e9 servers",
				connectSrc: ["https://*.xn--bcher-kva.de", "https://a.example.com"],
				imgSrc: ["https://a.example.com", "https://b.example.com"],
			},
			{
				reason: "Loads runtime tiles",
				inputs: [
					{
						path: "a",
						directives: ["mediaSrc"],
						template: { subdomains: ["a", "b"] },
					},
					{ path: "m", directives: ["imgSrc"] },
					{ path: "z", directives: ["connectSrc", "imgSrc"] },
				],
			},
		]);
		expect(Object.keys(normalized[0] ?? {})).toEqual([
			"reason",
			"connectSrc",
			"imgSrc",
		]);
	});

	test("an empty csp canonicalizes to a v1 contract without csp", () => {
		const canonical = canonicalizeContract(
			withDefaults({ contractVersion: 2, id: "plain", csp: [] }),
		);
		expect(canonical.contractVersion).toBe(1);
		expect(canonical).not.toHaveProperty("csp");
		expect(
			validateContract(
				withDefaults({ contractVersion: 2, id: "live-map", csp: [] }),
			),
		).toEqual([
			"Widget 'live-map' declares an empty csp; omit csp when it declares no purposes",
		]);
	});

	test("count a source once per directive across purposes and cap the bytes", () => {
		const hosts = Array.from(
			{ length: 16 },
			(_, index) => `https://h${String(index).padStart(2, "0")}.example.org`,
		);
		const atCap = withDefaults({
			contractVersion: 2,
			id: "live-map",
			csp: [
				{ reason: "Loads map tiles", connectSrc: hosts.slice(0, 8) },
				{ reason: "Loads map labels", imgSrc: hosts.slice(8) },
			],
		});
		expect(validateContract(atCap)).toEqual([]);
		expect(
			validateContract({
				...atCap,
				csp: [
					...(atCap.csp ?? []),
					{ reason: "Loads web fonts", fontSrc: ["https://fonts.example.org"] },
				],
			}),
		).toEqual([
			"Widget 'live-map': csp declares 17 sources; at most 16 are allowed",
		]);
	});
});

describe("network input paths", () => {
	test("parse into segments like parse_widget_input_path", () => {
		expect(parseWidgetInputPath("config.layers[].sources.*.url")).toEqual({
			root: "config",
			segments: [
				{ kind: "key", key: "layers" },
				{ kind: "items" },
				{ kind: "key", key: "sources" },
				{ kind: "values" },
				{ kind: "key", key: "url" },
			],
		});
		expect(parseWidgetInputPath("tileUrl")).toEqual({
			root: "tileUrl",
			segments: [],
		});
		expect(parseWidgetInputPath("_a[][]")).toEqual({
			root: "_a",
			segments: [{ kind: "items" }, { kind: "items" }],
		});
	});

	test("reject what Rust rejects", () => {
		for (const invalid of [
			"",
			"1a",
			"a.",
			"a..b",
			"a[0]",
			"a[",
			"a.*b",
			"a*",
			"a.1b",
			".a",
			"a b",
			"a-b",
			"a.b-c",
			"ä",
		]) {
			expect([invalid, parseWidgetInputPath(invalid)]).toEqual([invalid, null]);
			expect(isValidWidgetInputPath(invalid)).toBeFalse();
		}
		expect(isValidWidgetInputPath("a.b.c.d.e.f")).toBeTrue();
		expect(isValidWidgetInputPath("a.b.c.d.e.f.g")).toBeFalse();
	});
});
