import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	reasonCharactersAllowed,
	reasonContainsAddress,
	reasonHasMixedScript,
} from "../src/csp-reason";
import {
	PUBLIC_SUFFIX_RULES,
	WIDGET_SOURCE_CATALOG,
	WIDGET_SOURCE_DATA_PIN,
} from "../src/generated/widget-source-data";
import {
	isIcannBelow,
	isIcannSuffix,
	isIcannTld,
	validateWildcardBases,
} from "../src/psl";

const SCHEMA_DIR = join(import.meta.dir, "..", "..", "wasm", "schema");
const DATA_DIR = join(SCHEMA_DIR, "data");
const REGENERATE = "run `mise run widget-sources:generate`";

const CLASSIFICATION: {
	pslVersion: string;
	catalogVersion: number;
	wildcardBases: { accepted: string[]; rejected: string[] };
} = JSON.parse(
	readFileSync(
		join(SCHEMA_DIR, "tests", "fixtures", "widget_source_classification.json"),
		"utf8",
	),
);

function sha256Hex(text: string): string {
	return createHash("sha256").update(text).digest("hex");
}

function canonicalJson(value: unknown): string {
	if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
	if (value !== null && typeof value === "object") {
		const entries = Object.entries(value)
			.filter(([, entry]) => entry !== undefined)
			.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
		return `{${entries
			.map(([key, entry]) => `${JSON.stringify(key)}:${canonicalJson(entry)}`)
			.join(",")}}`;
	}
	return JSON.stringify(value);
}

function expectPinned(what: string, actual: unknown, pinned: unknown): void {
	if (actual !== pinned) {
		throw new Error(
			`${what} is ${JSON.stringify(actual)} but the pin says ${JSON.stringify(pinned)}; ${REGENERATE}`,
		);
	}
}

describe("generated widget source data", () => {
	test("PUBLIC_SUFFIX_RULES matches its pin and the schema data file", () => {
		expectPinned(
			"sha256 of PUBLIC_SUFFIX_RULES",
			sha256Hex(PUBLIC_SUFFIX_RULES),
			WIDGET_SOURCE_DATA_PIN.rulesSha256,
		);
		const rulesFile = readFileSync(
			join(DATA_DIR, "public_suffix_rules.txt"),
			"utf8",
		);
		expectPinned(
			"sha256 of packages/wasm/schema/data/public_suffix_rules.txt",
			sha256Hex(rulesFile),
			WIDGET_SOURCE_DATA_PIN.rulesSha256,
		);
		expectPinned(
			"psl-version of public_suffix_rules.txt",
			/^# psl-version: (\S+)$/m.exec(rulesFile)?.[1],
			WIDGET_SOURCE_DATA_PIN.pslVersion,
		);
	});

	test("the providers hash matches its pin and the schema catalog", () => {
		expectPinned(
			"sha256 of the generated providers",
			sha256Hex(canonicalJson(WIDGET_SOURCE_CATALOG.providers)),
			WIDGET_SOURCE_DATA_PIN.providersSha256,
		);
		const catalog = JSON.parse(
			readFileSync(join(DATA_DIR, "widget_source_catalog.json"), "utf8"),
		);
		expectPinned(
			"sha256 of the widget_source_catalog.json providers",
			sha256Hex(canonicalJson(catalog.providers)),
			WIDGET_SOURCE_DATA_PIN.providersSha256,
		);
		expectPinned(
			"widget_source_catalog.json providersSha256",
			catalog.providersSha256,
			WIDGET_SOURCE_DATA_PIN.providersSha256,
		);
		expectPinned(
			"widget_source_catalog.json catalogVersion",
			catalog.catalogVersion,
			WIDGET_SOURCE_DATA_PIN.catalogVersion,
		);
		expectPinned(
			"widget_source_catalog.json publicSuffixRules.sha256",
			catalog.publicSuffixRules?.sha256,
			WIDGET_SOURCE_DATA_PIN.rulesSha256,
		);
		expectPinned(
			"widget_source_catalog.json publicSuffixRules.pslVersion",
			catalog.publicSuffixRules?.pslVersion,
			WIDGET_SOURCE_DATA_PIN.pslVersion,
		);
	});

	test("the classification fixture was generated from the same data", () => {
		expectPinned(
			"widget_source_classification.json pslVersion",
			CLASSIFICATION.pslVersion,
			WIDGET_SOURCE_DATA_PIN.pslVersion,
		);
		expectPinned(
			"widget_source_classification.json catalogVersion",
			CLASSIFICATION.catalogVersion,
			WIDGET_SOURCE_DATA_PIN.catalogVersion,
		);
	});

	test("catalog invariants that depend on the TypeScript rules hold", () => {
		const providers = WIDGET_SOURCE_CATALOG.providers;
		expect(providers.length).toBeLessThanOrEqual(300);
		const ids = new Set<string>();
		const matches = new Set<string>();
		const problems: string[] = [];
		for (const provider of providers) {
			if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(provider.id)) {
				problems.push(`${provider.id}: id is not kebab-case`);
			}
			if (ids.has(provider.id)) problems.push(`${provider.id}: id repeats`);
			ids.add(provider.id);
			const nameLength = Array.from(provider.name).length;
			if (
				nameLength < 1 ||
				nameLength > 40 ||
				!reasonCharactersAllowed(provider.name) ||
				reasonHasMixedScript(provider.name) ||
				reasonContainsAddress(provider.name)
			) {
				problems.push(`${provider.id}: name ${JSON.stringify(provider.name)}`);
			}
			for (const match of provider.match) {
				const key = `${match.type} ${match.domain}`;
				if (matches.has(key)) problems.push(`${provider.id}: ${key} repeats`);
				matches.add(key);
				const star = match.domain.lastIndexOf("*.");
				const literal = star < 0 ? match.domain : match.domain.slice(star + 2);
				if (isIcannSuffix(literal)) {
					problems.push(`${provider.id}: ${match.domain} is a public suffix`);
				}
			}
		}
		expect(problems).toEqual([]);
	});
});

describe("public suffix checks", () => {
	test("validateWildcardBases follows the shared fixture", () => {
		for (const source of CLASSIFICATION.wildcardBases.accepted) {
			expect([source, validateWildcardBases([source])]).toEqual([source, []]);
		}
		for (const source of CLASSIFICATION.wildcardBases.rejected) {
			expect([source, validateWildcardBases([source])]).toEqual([
				source,
				[
					`Invalid csp source "${source}": wildcard base is a public suffix or spans public suffixes`,
				],
			]);
		}
	});

	test("ICANN primitives", () => {
		expect(isIcannSuffix("co.uk")).toBeTrue();
		expect(isIcannSuffix("foo.ck")).toBeTrue();
		expect(isIcannSuffix("www.ck")).toBeFalse();
		expect(isIcannSuffix("example.co.uk")).toBeFalse();
		expect(isIcannBelow("kawasaki.jp")).toBeTrue();
		expect(isIcannBelow("city.kawasaki.jp")).toBeFalse();
		expect(isIcannTld("com")).toBeTrue();
		expect(isIcannTld("xn--p1ai")).toBeTrue();
		expect(isIcannTld("js")).toBeFalse();
	});
});
