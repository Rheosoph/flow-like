import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { generate } from "./generate";
import {
	CATALOG_PATH,
	PSL_URL,
	RULES_PATH,
	blockName,
	parsePsl,
	readCatalog,
	readWaivers,
	renderRulesFile,
	rulesFileVersion,
	uncoveredBlocks,
} from "./lib";

const MIN_ICANN_RULES = 5000;
const MIN_PRIVATE_RULES = 2000;
const ANCHOR_RULES = ["s3.amazonaws.com", "cloudfront.net", "github.io"];

function fail(message: string): never {
	console.error(`widget-sources:update-psl: ${message}`);
	process.exit(1);
}

async function fetchList(): Promise<string> {
	const response = await fetch(PSL_URL);
	if (!response.ok) {
		fail(`fetching ${PSL_URL} failed with HTTP ${response.status}`);
	}
	return response.text();
}

const text = await fetchList();
let psl: ReturnType<typeof parsePsl>;
try {
	psl = parsePsl(text);
} catch (error) {
	fail((error as Error).message);
}

const current = existsSync(RULES_PATH)
	? rulesFileVersion(readFileSync(RULES_PATH, "utf8"))
	: undefined;
if (current && psl.version <= current) {
	fail(`fetched VERSION ${psl.version} is not newer than ${current}`);
}

const icann = psl.rules.filter((rule) => rule.section === "i").length;
const privateRules = psl.rules.filter((rule) => rule.section === "p");
if (icann < MIN_ICANN_RULES) {
	fail(`only ${icann} ICANN rules (expected at least ${MIN_ICANN_RULES})`);
}
if (privateRules.length < MIN_PRIVATE_RULES) {
	fail(
		`only ${privateRules.length} private rules (expected at least ${MIN_PRIVATE_RULES})`,
	);
}
const missingAnchors = ANCHOR_RULES.filter(
	(anchor) => !privateRules.some((rule) => rule.rule === anchor),
);
if (missingAnchors.length > 0) {
	fail(`anchor rules missing: ${missingAnchors.join(", ")}`);
}

if (!existsSync(CATALOG_PATH)) {
	fail(`${CATALOG_PATH} is missing`);
}
const uncovered = uncoveredBlocks(psl.blocks, readCatalog(), readWaivers());
if (uncovered.length > 0) {
	const list = uncovered
		.map(
			(block) => `  ${blockName(block.heading)} (${block.rules.length} rules)`,
		)
		.join("\n");
	fail(
		`PSL private operator blocks without catalog attribution; add a catalog entry or a line to data/attribution_waivers.txt:\n${list}`,
	);
}

writeFileSync(RULES_PATH, renderRulesFile(psl));
console.log(
	`widget-sources:update-psl: wrote PSL ${psl.version} (${icann} ICANN, ${privateRules.length} private rules)`,
);
generate();
