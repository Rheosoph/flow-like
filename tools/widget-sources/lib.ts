import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { domainToASCII } from "node:url";

export const REPO_ROOT = join(import.meta.dir, "..", "..");
export const DATA_DIR = join(REPO_ROOT, "packages/wasm/schema/data");
export const RULES_PATH = join(DATA_DIR, "public_suffix_rules.txt");
export const CATALOG_PATH = join(DATA_DIR, "widget_source_catalog.json");
export const WAIVERS_PATH = join(DATA_DIR, "attribution_waivers.txt");
export const GENERATED_TS_PATH = join(
	REPO_ROOT,
	"packages/widget-bundler/src/generated/widget-source-data.ts",
);
export const PSL_URL = "https://publicsuffix.org/list/public_suffix_list.dat";

const ICANN_BEGIN = "// ===BEGIN ICANN DOMAINS===";
const ICANN_END = "// ===END ICANN DOMAINS===";
const PRIVATE_BEGIN = "// ===BEGIN PRIVATE DOMAINS===";
const PRIVATE_END = "// ===END PRIVATE DOMAINS===";

export type Section = "i" | "p";

export interface PslRule {
	section: Section;
	rule: string;
}

export interface PrivateBlock {
	heading: string;
	rules: string[];
}

export interface ParsedPsl {
	version: string;
	commit?: string;
	sourceSha256: string;
	rules: PslRule[];
	blocks: PrivateBlock[];
}

export interface CatalogMatch {
	type: string;
	domain: string;
}

export interface CatalogProvider {
	id: string;
	name: string;
	aboutKey?: string;
	docs: string[];
	match: CatalogMatch[];
	receives?: boolean;
	userContent?: boolean;
}

export interface Catalog {
	catalogVersion: number;
	providersSha256: string;
	publicSuffixRules: { pslVersion: string; sha256: string };
	providers: CatalogProvider[];
}

export function sha256Hex(data: string | Uint8Array): string {
	return createHash("sha256").update(data).digest("hex");
}

export function canonicalJson(value: unknown): string {
	if (Array.isArray(value)) {
		return `[${value.map(canonicalJson).join(",")}]`;
	}
	if (value !== null && typeof value === "object") {
		const entries = Object.entries(value as Record<string, unknown>)
			.filter(([, entry]) => entry !== undefined)
			.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
		return `{${entries
			.map(([key, entry]) => `${JSON.stringify(key)}:${canonicalJson(entry)}`)
			.join(",")}}`;
	}
	return JSON.stringify(value);
}

export function providersSha256(providers: CatalogProvider[]): string {
	return sha256Hex(canonicalJson(providers));
}

function headerValue(text: string, key: string): string | undefined {
	const match = text.match(new RegExp(`^// ${key}: (\\S+)$`, "m"));
	return match?.[1];
}

function toAsciiRule(raw: string): string {
	const prefix = raw.startsWith("!") ? "!" : raw.startsWith("*.") ? "*." : "";
	const base = raw.slice(prefix.length);
	const ascii = domainToASCII(base);
	if (!ascii || !/^[a-z0-9-]+(\.[a-z0-9-]+)*$/.test(ascii)) {
		throw new Error(
			`cannot convert PSL rule "${raw}" to ASCII (got "${ascii}")`,
		);
	}
	return `${prefix}${ascii}`;
}

export function parsePsl(text: string): ParsedPsl {
	const lines = text.split("\n");
	const markers = [ICANN_BEGIN, ICANN_END, PRIVATE_BEGIN, PRIVATE_END].map(
		(marker) => lines.findIndex((line) => line.trim() === marker),
	);
	const missing = markers.findIndex((index) => index < 0);
	if (missing >= 0) {
		throw new Error(
			`PSL section marker missing: ${[ICANN_BEGIN, ICANN_END, PRIVATE_BEGIN, PRIVATE_END][missing]}`,
		);
	}
	const [icannBegin, icannEnd, privateBegin, privateEnd] = markers;
	if (
		!(
			icannBegin < icannEnd &&
			icannEnd < privateBegin &&
			privateBegin < privateEnd
		)
	) {
		throw new Error("PSL section markers are out of order");
	}
	const version = headerValue(text, "VERSION");
	if (!version) {
		throw new Error("PSL VERSION header missing");
	}

	const rules: PslRule[] = [];
	const blocks: PrivateBlock[] = [];
	let current: PrivateBlock | undefined;
	let lastWasRule = false;
	for (let index = icannBegin + 1; index < privateEnd; index++) {
		if (index >= icannEnd && index <= privateBegin) {
			continue;
		}
		const section: Section = index < icannEnd ? "i" : "p";
		const line = lines[index].trim();
		if (line === "") {
			lastWasRule = false;
			current = undefined;
			continue;
		}
		if (line.startsWith("//")) {
			if (section === "p" && (!current || lastWasRule)) {
				current = { heading: line.replace(/^\/\/\s*/, ""), rules: [] };
				blocks.push(current);
			}
			lastWasRule = false;
			continue;
		}
		const rule = toAsciiRule(line.split(/\s/)[0].toLowerCase());
		rules.push({ section, rule });
		if (section === "p" && current) {
			current.rules.push(rule);
		}
		lastWasRule = true;
	}

	return {
		version,
		commit: headerValue(text, "COMMIT"),
		sourceSha256: sha256Hex(text),
		rules,
		blocks,
	};
}

export function renderRulesFile(psl: ParsedPsl): string {
	const header = [
		"# Public Suffix List rules for widget source classification (spec 14.3.2).",
		`# psl-version: ${psl.version}`,
		`# psl-commit: ${psl.commit ?? "unknown"}`,
		`# psl-source-sha256: ${psl.sourceSha256}`,
		`# psl-source: ${PSL_URL}`,
		"# One rule per line: 'i ' for the ICANN section, 'p ' for the private section.",
		"# Non-ASCII rules are converted with UTS-46 (punycode); upstream order is kept.",
		"#",
		"# This Source Code Form is subject to the terms of the Mozilla Public",
		"# License, v. 2.0. If a copy of the MPL was not distributed with this",
		"# file, You can obtain one at https://mozilla.org/MPL/2.0/.",
	];
	const body = psl.rules.map(({ section, rule }) => `${section} ${rule}`);
	return `${[...header, ...body].join("\n")}\n`;
}

export function rulesFileVersion(text: string): string | undefined {
	return text.match(/^# psl-version: (\S+)$/m)?.[1];
}

export function readCatalog(): Catalog {
	return JSON.parse(readFileSync(CATALOG_PATH, "utf8")) as Catalog;
}

export function writeCatalog(catalog: Catalog): void {
	const text = JSON.stringify(catalog, null, 2).replace(
		/\{\n\s+"type": ("[^"]*"),\n\s+"domain": ("[^"]*")\n\s+\}/g,
		'{ "type": $1, "domain": $2 }',
	);
	writeFileSync(CATALOG_PATH, `${text}\n`);
}

export function readWaivers(): Set<string> {
	const text = readFileSync(WAIVERS_PATH, "utf8");
	return new Set(
		text
			.split("\n")
			.map((line) => line.trim())
			.filter((line) => line !== "" && !line.startsWith("#")),
	);
}

export function blockName(heading: string): string {
	return heading.split(" : ")[0].trim();
}

function labelsMatch(pattern: string[], labels: string[]): boolean {
	return (
		pattern.length === labels.length &&
		pattern.every((label, index) => label === "*" || label === labels[index])
	);
}

function atOrUnder(domain: string, pattern: string): boolean {
	const labels = domain.split(".");
	const patternLabels = pattern.split(".");
	return (
		labels.length >= patternLabels.length &&
		labelsMatch(
			patternLabels,
			labels.slice(labels.length - patternLabels.length),
		)
	);
}

export function blockCovered(block: PrivateBlock, catalog: Catalog): boolean {
	const domains = catalog.providers.flatMap((provider) =>
		provider.match.map((match) => match.domain),
	);
	return block.rules.some((rule) => {
		const base = rule.replace(/^!/, "").replace(/^\*\./, "");
		return domains.some((domain) => atOrUnder(base, domain));
	});
}

export const MIN_BLOCK_RULES = 10;

export function uncoveredBlocks(
	blocks: PrivateBlock[],
	catalog: Catalog,
	waivers: Set<string>,
): PrivateBlock[] {
	return blocks.filter(
		(block) =>
			block.rules.length >= MIN_BLOCK_RULES &&
			!waivers.has(blockName(block.heading)) &&
			!blockCovered(block, catalog),
	);
}
