import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Glob } from "bun";

const PACKAGE_ROOT = join(import.meta.dir, "..");

/** The roots `global.css` hands to Tailwind's `@source`, test files included. */
const SCANNED = new Glob(
	"{components,hooks,lib,state,pages,src,app}/**/*.{ts,tsx}",
);

/**
 * Class tokens that compile to a `:has()` Chromium can't scope: `:root:has(…) *`, or `:has()` nested
 * in `:is(:where(…))` (`group-has-*`, `group-[:has(…)]`). Each DOM change then restyles the whole
 * document; typing into the bot editor took ~55 ms a keystroke with ~7k elements on the page.
 * Write the group as an arbitrary variant instead: `[[data-x]:has(…)_&]:utility`.
 */
const FORBIDDEN: readonly { name: string; pattern: RegExp }[] = [
	{ name: "group-has-*", pattern: /\bgroup-has-/ },
	{ name: "group-[:has()]", pattern: /\bgroup-\[[^\]\s]*:has\(/ },
	{ name: "[:root:has()]", pattern: /\[[^\]\s]*:root:has\([^\]\s]*\]:/ },
];

function offenders() {
	const found: string[] = [];
	for (const path of SCANNED.scanSync(PACKAGE_ROOT)) {
		const file = join(PACKAGE_ROOT, path);
		if (file === import.meta.path) continue;
		const lines = readFileSync(file, "utf8").split("\n");
		lines.forEach((line, index) => {
			for (const { name, pattern } of FORBIDDEN)
				if (pattern.test(line)) found.push(`${path}:${index + 1} ${name}`);
		});
	}
	return found;
}

describe("Tailwind :has() variants", () => {
	test("no class compiles to a :has() that restyles the whole document", () => {
		expect(offenders()).toEqual([]);
	});
});
