import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
	CSP_REASON_REJECTION_MESSAGES,
	type WidgetCspReasonRejection,
	foldWidgetCspReason,
	isDefaultIgnorable,
	normalizeCspReason,
	reasonContainsAddress,
	validateWidgetCspReason,
} from "../src/csp-reason";

const FIXTURES = join(
	import.meta.dir,
	"..",
	"..",
	"wasm",
	"schema",
	"tests",
	"fixtures",
);

interface ReasonCase {
	reason: string;
	hasInputs: boolean;
	code?: WidgetCspReasonRejection;
}

const CSP_FIXTURE: {
	unicodeVersion: string;
	acceptedReasons: ReasonCase[];
	rejectedReasons: ReasonCase[];
	duplicateReasons: { reasons: [string, string]; duplicate: boolean }[];
} = JSON.parse(readFileSync(join(FIXTURES, "widget_csp.json"), "utf8"));

const ADDRESS_FIXTURE: {
	reasonAddress: { accepted: string[]; rejected: string[] };
} = JSON.parse(
	readFileSync(join(FIXTURES, "widget_source_classification.json"), "utf8"),
);

function versionAtLeast(actual: string, pinned: string): boolean {
	const left = actual.split(".").map(Number);
	const right = pinned.split(".").map(Number);
	for (let index = 0; index < right.length; index++) {
		const difference = (left[index] ?? 0) - (right[index] ?? 0);
		if (difference !== 0) return difference > 0;
	}
	return true;
}

describe("shared reason fixture", () => {
	test("the runtime's Unicode tables cover the pinned version", () => {
		const runtime = process.versions.unicode;
		if (runtime !== undefined) {
			expect([
				runtime,
				versionAtLeast(runtime, CSP_FIXTURE.unicodeVersion),
			]).toEqual([runtime, true]);
		}
		const assignedOnly = [
			...CSP_FIXTURE.acceptedReasons,
			...CSP_FIXTURE.rejectedReasons,
		]
			.map((entry) => entry.reason)
			.join("");
		expect(/\p{Cn}/u.test(assignedOnly)).toBeFalse();
	});

	test("accepts every accepted reason", () => {
		for (const entry of CSP_FIXTURE.acceptedReasons) {
			expect([
				entry.reason,
				validateWidgetCspReason(entry.reason, entry.hasInputs),
			]).toEqual([entry.reason, null]);
		}
	});

	test("rejects every rejected reason with the listed code", () => {
		expect(CSP_FIXTURE.rejectedReasons.length).toBeGreaterThanOrEqual(30);
		for (const entry of CSP_FIXTURE.rejectedReasons) {
			expect([
				entry.reason,
				validateWidgetCspReason(entry.reason, entry.hasInputs),
			]).toEqual([entry.reason, entry.code ?? null]);
			expect(
				CSP_REASON_REJECTION_MESSAGES[entry.code as WidgetCspReasonRejection],
			).toBeString();
		}
	});

	test("folds duplicate reasons like Rust", () => {
		for (const entry of CSP_FIXTURE.duplicateReasons) {
			const [first, second] = entry.reasons;
			expect([
				entry.reasons,
				foldWidgetCspReason(first) === foldWidgetCspReason(second),
			]).toEqual([entry.reasons, entry.duplicate]);
		}
	});
});

describe("foldWidgetCspReason", () => {
	test("matches the Rust folding cases", () => {
		expect(
			foldWidgetCspReason("\uff2c\uff4f\uff41\uff44\uff53 \uff2d\uff21\uff30"),
		).toBe("loads map");
		expect(foldWidgetCspReason("cesium\u3002com")).toBe("cesium.com");
		expect(foldWidgetCspReason("\u5730\u56f3\u3002com")).toBe(
			"\u5730\u56f3\u3002com",
		);
	});

	test("drops marks and default-ignorable code points like Rust", () => {
		expect(
			foldWidgetCspReason("V\u00c9r\u034fified\ufe0f Caf\u00e9\u3164"),
		).toBe("verified cafe");
		expect(foldWidgetCspReason("\uffa0\u115f\u1160")).toBe("");
		expect(
			foldWidgetCspReason(
				"\uff2c\uff4f\uff41\uff44\uff53\u{e0100} \uff4d\u00e1p",
			),
		).toBe(foldWidgetCspReason("loads map"));
	});

	test("the default-ignorable ranges equal the engine's Unicode property", () => {
		const property = /^\p{Default_Ignorable_Code_Point}$/u;
		const mismatches: string[] = [];
		for (let code = 0; code <= 0x10ffff; code++) {
			if (code >= 0xd800 && code <= 0xdfff) continue;
			const c = String.fromCodePoint(code);
			if (isDefaultIgnorable(c) !== property.test(c)) {
				mismatches.push(code.toString(16));
			}
		}
		expect(mismatches).toEqual([]);
	});

	test("over-long reasons fail on length before other rules", () => {
		const cases = [
			"\u{1f680}".repeat(121),
			`Loads tiles! ${"a".repeat(120)}`,
			`Cafe\u0301 ${"b".repeat(120)}`,
			`Loads ${"\ud800".repeat(115)}`,
		];
		for (const reason of cases) {
			expect([reason, validateWidgetCspReason(reason, false)]).toEqual([
				reason,
				"reason-length",
			]);
		}
		expect(
			validateWidgetCspReason(`Loads ${"a".repeat(114)}`, false),
		).toBeNull();
	});

	test("joiners are allowed only between non-ASCII letters", () => {
		expect(
			validateWidgetCspReason(
				"\u06a9\u062a\u0627\u0628\u200c\u0647\u0627 load",
				false,
			),
		).toBeNull();
		expect(validateWidgetCspReason("Loads\u200ctiles", false)).toBe(
			"reason-forbidden-character",
		);
		expect(
			validateWidgetCspReason(
				"\u200c\u0646\u0642\u0634\u0647\u200c\u0647\u0627 \u0627\u0632 \u0633\u0631\u0648\u0631",
				false,
			),
		).toBe("reason-forbidden-character");
		expect(validateWidgetCspReason("Loads \ud800 tiles", false)).toBe(
			"reason-forbidden-character",
		);
	});
});

describe("reasonContainsAddress", () => {
	test("follows the shared fixture", () => {
		for (const reason of ADDRESS_FIXTURE.reasonAddress.accepted) {
			expect([reason, reasonContainsAddress(reason)]).toEqual([reason, false]);
		}
		for (const reason of ADDRESS_FIXTURE.reasonAddress.rejected) {
			expect([reason, reasonContainsAddress(reason)]).toEqual([reason, true]);
		}
	});

	test("accepted fixture reasons mention no address", () => {
		for (const entry of CSP_FIXTURE.acceptedReasons) {
			expect([entry.reason, reasonContainsAddress(entry.reason)]).toEqual([
				entry.reason,
				false,
			]);
		}
	});
});

describe("normalizeCspReason", () => {
	test("composes to NFC, collapses whitespace and trims", () => {
		expect(
			normalizeCspReason("  Loads\tmap\u00a0tiles \n from Cafe\u0301 "),
		).toBe("Loads map tiles from Caf\u00e9");
	});
});
