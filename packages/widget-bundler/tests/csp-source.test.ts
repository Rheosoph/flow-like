import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	CSP_DIRECTIVES,
	CSP_SOURCE_REJECTION_MESSAGES,
	type CspDirective,
	type CspSourceRejection,
	MAX_WIDGET_CSP_SOURCES,
	compareCspSources,
	isCspDirective,
	normalizeCspDeclaration,
	normalizeCspSource,
	validateCspDeclaration,
	validateCspSource,
} from "../src/csp-source";

interface SourceCase {
	directive: string;
	source: string;
	reason?: string;
}

interface CspFixture {
	acceptedSources: SourceCase[];
	rejectedSources: SourceCase[];
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
			["https://*.Example.com", "wildcard"],
			["'self'", "keyword"],
			["Https:api.maptiler.com", "missing-scheme"],
		] as const) {
			expect([
				source,
				validateCspSource("connectSrc", normalizeCspSource(source)),
			]).toEqual([source, reason]);
		}
	});
});

describe("normalizeCspDeclaration", () => {
	test("sorts, deduplicates and drops empty directives", () => {
		expect(
			normalizeCspDeclaration({
				styleSrc: ["https://fonts.googleapis.com"],
				imgSrc: [
					"https://B.tile.openstreetmap.org",
					"https://a.tile.openstreetmap.org",
					"https://b.tile.openstreetmap.org",
				],
				connectSrc: ["wss://live.example.com", "HTTPS://api.maptiler.com"],
				mediaSrc: [],
			}),
		).toEqual({
			connectSrc: ["https://api.maptiler.com", "wss://live.example.com"],
			imgSrc: [
				"https://a.tile.openstreetmap.org",
				"https://b.tile.openstreetmap.org",
			],
			styleSrc: ["https://fonts.googleapis.com"],
		});
	});

	test("emits directives in serde field order", () => {
		const normalized = normalizeCspDeclaration({
			styleSrc: ["https://s.example.org"],
			mediaSrc: ["https://m.example.org"],
			fontSrc: ["https://f.example.org"],
			imgSrc: ["https://i.example.org"],
			connectSrc: ["https://c.example.org"],
		});
		expect(Object.keys(normalized)).toEqual([...CSP_DIRECTIVES]);
	});

	test("an all-empty declaration normalizes to nothing", () => {
		expect(normalizeCspDeclaration({ connectSrc: [], imgSrc: [] })).toEqual({});
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

describe("validateCspDeclaration", () => {
	test("accepts a canonical declaration", () => {
		expect(
			validateCspDeclaration({
				connectSrc: ["https://api.maptiler.com", "wss://live.example.com"],
				imgSrc: ["https://a.tile.openstreetmap.org"],
			}),
		).toEqual([]);
	});

	test("reports grammar errors in the Rust message format", () => {
		expect(
			validateCspDeclaration({
				connectSrc: ["http://api.example.org"],
				imgSrc: ["wss://tiles.example.org"],
			}),
		).toEqual([
			'Invalid csp source "http://api.example.org" in connectSrc: scheme is not allowed for this directive',
			'Invalid csp source "wss://tiles.example.org" in imgSrc: scheme is not allowed for this directive',
		]);
	});

	test("requires ascending order without duplicates", () => {
		expect(
			validateCspDeclaration({
				connectSrc: ["https://b.example.org", "https://a.example.org"],
			}),
		).toEqual(["csp connectSrc must be sorted ascending without duplicates"]);
		expect(
			validateCspDeclaration({
				fontSrc: ["https://a.example.org", "https://a.example.org"],
			}),
		).toEqual(["csp fontSrc must be sorted ascending without duplicates"]);
	});

	test("caps the total number of sources", () => {
		const sources = Array.from(
			{ length: MAX_WIDGET_CSP_SOURCES },
			(_, index) => `https://h${String(index).padStart(2, "0")}.example.org`,
		);
		const atCap = {
			connectSrc: sources.slice(0, 8),
			imgSrc: sources.slice(8),
		};
		expect(validateCspDeclaration(atCap)).toEqual([]);
		expect(
			validateCspDeclaration({
				...atCap,
				fontSrc: ["https://fonts.example.org"],
			}),
		).toEqual(["csp declares 17 sources; at most 16 are allowed"]);
	});

	test("refuses shapes serde would not parse", () => {
		expect(
			validateCspDeclaration({ scriptSrc: ["https://cdn.example.org"] }),
		).toEqual([
			'csp declares unknown directive "scriptSrc" (allowed: connectSrc, imgSrc, fontSrc, mediaSrc, styleSrc)',
		]);
		expect(
			validateCspDeclaration({
				connectSrc: "https://api.example.org",
				imgSrc: [1],
				fontSrc: null,
			}),
		).toEqual([
			"csp connectSrc must be an array of strings",
			"csp imgSrc must be an array of strings",
			"csp fontSrc must be an array of strings",
		]);
		expect(validateCspDeclaration(["https://api.example.org"])).toEqual([
			"csp must be an object",
		]);
	});
});
