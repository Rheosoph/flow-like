import { describe, expect, test } from "bun:test";
import {
	FLOWSCRIPT_CLAIMS_FIELD,
	FLOWSCRIPT_CURSOR_FIELD,
	FLOWSCRIPT_SCOPE_FIELD,
	MAX_CLAIM_ANCHORS,
	MAX_SCOPE_NODES,
	MAX_WIRE_COLUMN,
	MAX_WIRE_DLINE,
	sanitizeClaimsForWire,
	sanitizeCursorForWire,
	sanitizeForWire,
	sanitizeScopeForWire,
	wireSafetyViolations,
} from "./flowscript-presence-protocol";

const VALID_ID = "j37yxnwykc08y10reikmija1";
const OTHER_ID = "wjapgelphq6vh3pr4jtm403n";

const validCursor = () => ({
	anchor: { id: VALID_ID, kind: "node" },
	dLine: 2,
	column: 14,
	sel: { endAnchorId: OTHER_ID, endDLine: 0, endColumn: 3 },
	ts: 1_700_000_000_000,
});

describe("FlowScript presence wire schema (collab rule 2)", () => {
	test("passes a valid cursor payload through unchanged", () => {
		const sanitized = sanitizeCursorForWire(validCursor());
		expect(sanitized).toEqual(validCursor() as never);
	});

	test("rejects cursor payloads whose anchor id could carry text", () => {
		for (const id of [
			"const secret = loadKey()", // code text
			"a".repeat(64), // unbounded
			"short", // below id length floor
			"has spaces here", // not id-shaped
			"", // empty
			42, // wrong type
		]) {
			expect(
				sanitizeCursorForWire({
					...validCursor(),
					anchor: { id, kind: "node" },
				}),
			).toBeUndefined();
		}
	});

	test("rejects unknown anchor kinds", () => {
		expect(
			sanitizeCursorForWire({
				...validCursor(),
				anchor: { id: VALID_ID, kind: "freeform text" },
			}),
		).toBeUndefined();
	});

	test("clamps numeric fields into bounds and rejects non-finite ones", () => {
		const clamped = sanitizeCursorForWire({
			...validCursor(),
			dLine: 9_999,
			column: 123_456,
			ts: -5,
		});
		expect(clamped?.dLine).toBe(MAX_WIRE_DLINE);
		expect(clamped?.column).toBe(MAX_WIRE_COLUMN);
		expect(clamped?.ts).toBe(0);
		expect(
			sanitizeCursorForWire({ ...validCursor(), dLine: Number.NaN }),
		).toBeUndefined();
		expect(
			sanitizeCursorForWire({
				...validCursor(),
				column: Number.POSITIVE_INFINITY,
			}),
		).toBeUndefined();
	});

	test("drops a selection whose end anchor fails validation instead of guessing", () => {
		const sanitized = sanitizeCursorForWire({
			...validCursor(),
			sel: { endAnchorId: "not an id at all!", endDLine: 1, endColumn: 2 },
		});
		expect(sanitized).toBeDefined();
		expect(sanitized?.sel).toBeUndefined();
	});

	test("strips unknown keys — extra fields never reach the wire", () => {
		const sanitized = sanitizeCursorForWire({
			...validCursor(),
			note: "leaked code text: const x = 1",
			nested: { evil: "payload" },
		});
		expect(sanitized).toBeDefined();
		expect(Object.keys(sanitized as object).sort()).toEqual([
			"anchor",
			"column",
			"dLine",
			"sel",
			"ts",
		]);
	});

	test("claims: drops non-id entries, dedupes, and caps the set", () => {
		const sanitized = sanitizeClaimsForWire({
			anchorIds: [
				VALID_ID,
				VALID_ID,
				"function main() { return board }",
				...Array.from(
					{ length: 200 },
					(_, i) => `claimanchor${String(i).padStart(8, "0")}`,
				),
			],
			ts: 1,
		});
		expect(sanitized).toBeDefined();
		expect(sanitized?.anchorIds.length).toBe(MAX_CLAIM_ANCHORS);
		expect(sanitized?.anchorIds[0]).toBe(VALID_ID);
		expect(sanitized?.anchorIds.filter((id) => id === VALID_ID).length).toBe(1);
	});

	test("claims: an all-invalid or empty set publishes nothing", () => {
		expect(
			sanitizeClaimsForWire({ anchorIds: ["free text!", ""], ts: 1 }),
		).toBeUndefined();
		expect(sanitizeClaimsForWire({ anchorIds: [], ts: 1 })).toBeUndefined();
		expect(sanitizeClaimsForWire("anchorIds")).toBeUndefined();
	});

	test("sanitizeForWire dispatches by awareness field and rejects unknown fields", () => {
		expect(
			sanitizeForWire(FLOWSCRIPT_CURSOR_FIELD, validCursor()),
		).toBeDefined();
		expect(
			sanitizeForWire(FLOWSCRIPT_CLAIMS_FIELD, {
				anchorIds: [VALID_ID],
				ts: 1,
			}),
		).toBeDefined();
		expect(
			sanitizeForWire(FLOWSCRIPT_SCOPE_FIELD, {
				nodeIds: [VALID_ID],
				ts: 1,
			}),
		).toBeDefined();
		expect(
			// biome-ignore lint/suspicious/noExplicitAny: exercising the unknown-field path
			(sanitizeForWire as any)("chatMessage", { text: "hi" }),
		).toBeUndefined();
	});

	test("scope: passes a valid payload through with only known keys", () => {
		const sanitized = sanitizeScopeForWire({
			nodeIds: [VALID_ID, OTHER_ID],
			ts: 42,
			note: "leaked code text: const x = 1",
		});
		expect(sanitized).toEqual({ nodeIds: [VALID_ID, OTHER_ID], ts: 42 });
		expect(Object.keys(sanitized as object).sort()).toEqual(["nodeIds", "ts"]);
	});

	test("scope: drops non-id entries, dedupes, and caps the set", () => {
		const sanitized = sanitizeScopeForWire({
			nodeIds: [
				VALID_ID,
				VALID_ID,
				"function main() { return board }",
				...Array.from(
					{ length: 200 },
					(_, i) => `scopenode00${String(i).padStart(8, "0")}`,
				),
			],
			ts: 1,
		});
		expect(sanitized).toBeDefined();
		expect(sanitized?.nodeIds.length).toBe(MAX_SCOPE_NODES);
		expect(sanitized?.nodeIds[0]).toBe(VALID_ID);
		expect(sanitized?.nodeIds.filter((id) => id === VALID_ID).length).toBe(1);
	});

	test("scope: an all-invalid or empty set publishes nothing", () => {
		expect(
			sanitizeScopeForWire({ nodeIds: ["free text!", ""], ts: 1 }),
		).toBeUndefined();
		expect(sanitizeScopeForWire({ nodeIds: [], ts: 1 })).toBeUndefined();
		expect(sanitizeScopeForWire({ ts: 1 })).toBeUndefined();
		expect(sanitizeScopeForWire("nodeIds")).toBeUndefined();
		expect(sanitizeScopeForWire(undefined)).toBeUndefined();
	});

	test("scope: hostile payloads come out metadata-only or not at all", () => {
		const hostile: unknown[] = [
			{ nodeIds: [VALID_ID, "leak: pin values", OTHER_ID], ts: 123 },
			{ nodeIds: [VALID_ID], ts: Number.MAX_SAFE_INTEGER, evil: { a: "b" } },
			{ nodeIds: [VALID_ID], ts: -5 },
		];
		for (const payload of hostile) {
			const sanitized = sanitizeScopeForWire(payload);
			expect(sanitized).toBeDefined();
			expect(wireSafetyViolations(sanitized)).toEqual([]);
		}
	});

	test("scope: schema walk flags free text and oversized node lists", () => {
		expect(
			wireSafetyViolations({ nodeIds: ["some code fragment"], ts: 1 }),
		).not.toEqual([]);
		expect(
			wireSafetyViolations({
				nodeIds: Array.from({ length: 500 }, () => VALID_ID),
				ts: 1,
			}),
		).not.toEqual([]);
		expect(
			wireSafetyViolations({ nodeIds: [VALID_ID], ts: 1, scopeName: "Q3" }),
		).not.toEqual([]);
	});

	test("schema walk: every sanitizable hostile payload comes out metadata-only", () => {
		const hostile: unknown[] = [
			validCursor(),
			{ ...validCursor(), extra: "const flow = renderBoard()" },
			{
				anchor: { id: VALID_ID, kind: "node" },
				dLine: 1e9,
				column: -3,
				ts: Number.MAX_SAFE_INTEGER,
			},
			{
				anchorIds: [VALID_ID, "leak: pin values", OTHER_ID],
				ts: 123,
			},
		];
		for (const payload of hostile) {
			const asCursor = sanitizeCursorForWire(payload);
			if (asCursor) expect(wireSafetyViolations(asCursor)).toEqual([]);
			const asClaims = sanitizeClaimsForWire(payload);
			if (asClaims) expect(wireSafetyViolations(asClaims)).toEqual([]);
			expect(asCursor ?? asClaims).toBeDefined();
		}
	});

	test("schema walk: flags free text, unknown keys and oversized arrays", () => {
		expect(
			wireSafetyViolations({
				anchor: { id: VALID_ID, kind: "node" },
				dLine: 0,
				column: 1,
				ts: 1,
				snippet: "const leaked = true",
			}),
		).not.toEqual([]);
		expect(
			wireSafetyViolations({ anchorIds: ["some code fragment"], ts: 1 }),
		).not.toEqual([]);
		expect(
			wireSafetyViolations({ anchorIds: [`${"a".repeat(40)}`], ts: 1 }),
		).not.toEqual([]);
		expect(
			wireSafetyViolations({
				anchorIds: Array.from({ length: 500 }, () => VALID_ID),
				ts: 1,
			}),
		).not.toEqual([]);
		expect(wireSafetyViolations({ ts: Number.NaN })).not.toEqual([]);
		expect(wireSafetyViolations({ anchor: { id: () => "fn" } })).not.toEqual(
			[],
		);
	});
});
