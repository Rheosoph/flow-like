import { describe, expect, test } from "bun:test";
import {
	FOCUS_ATTR,
	type FocusRequest,
	type FocusTarget,
	type FormSessionState,
	type WorkbenchLayout,
} from "../contracts";
import { fixture } from "../testing/fixtures";
import { DESKTOP_LAYOUT, PHONE_LAYOUT } from "../testing/layouts";
import {
	QUIET_FOCUS_CSS,
	SELECTION_CSS,
	applyFocusRequest,
	autofocusKey,
	findFocusable,
	focusValueOf,
	resolveFocus,
} from "./focus";

describe("focusValueOf", () => {
	test("every target names the element it means", () => {
		const cases: readonly [FocusTarget, string][] = [
			[
				{ kind: "field", key: "vendor_name", select: false, scrollOnly: false },
				"field:vendor_name",
			],
			[{ kind: "tab", runId: "run-9" }, "tab:run-9"],
			[{ kind: "run" }, "run"],
			[{ kind: "runAgain" }, "run-again"],
			[{ kind: "stop" }, "stop"],
			[{ kind: "change" }, "change"],
			[{ kind: "copy" }, "copy"],
			[{ kind: "presetButton" }, "preset-button"],
			[{ kind: "filter" }, "filter"],
		];
		for (const [target, value] of cases)
			expect(focusValueOf(target)).toBe(value);
	});
});

interface FakeNode {
	readonly value: string;
	readonly disabled: boolean;
	focused: number;
	selected: number;
	scrolled: number;
	getAttribute(name: string): string | null;
	focus(): void;
	select(): void;
	scrollIntoView(): void;
}

function node(value: string, disabled = false): FakeNode {
	return {
		value,
		disabled,
		focused: 0,
		selected: 0,
		scrolled: 0,
		getAttribute(name) {
			return name === FOCUS_ATTR ? value : null;
		},
		focus() {
			this.focused += 1;
		},
		select() {
			this.selected += 1;
		},
		scrollIntoView() {
			this.scrolled += 1;
		},
	};
}

function rootOf(nodes: readonly FakeNode[]) {
	const root = {
		focused: 0,
		querySelectorAll: () => nodes,
		focus() {
			this.focused += 1;
		},
	};
	return root as unknown as HTMLElement & { focused: number };
}

const request = (target: FocusTarget): FocusRequest => ({ seq: 1, target });
const field = (
	key: string,
	extra: { select?: boolean; scrollOnly?: boolean } = {},
): FocusTarget => ({
	kind: "field",
	key,
	select: extra.select ?? false,
	scrollOnly: extra.scrollOnly ?? false,
});

describe("findFocusable and resolveFocus", () => {
	test("the first enabled element with the value, in document order", () => {
		const first = node("run", true);
		const second = node("run");
		const third = node("run");
		const root = rootOf([first, second, third]);
		expect(findFocusable(root, "run")).toBe(second as unknown as HTMLElement);
	});

	test("a field key with the object separator matches as written", () => {
		const grouped = node("field:payment_terms\u001fdays");
		expect(
			findFocusable(
				rootOf([node("field:other"), grouped]),
				"field:payment_terms\u001fdays",
			),
		).toBe(grouped as unknown as HTMLElement);
	});

	test("Stop falls back to Run, Run to Run again", () => {
		const run = node("run");
		const again = node("run-again");
		expect(resolveFocus(rootOf([run]), { kind: "stop" })).toBe(
			run as unknown as HTMLElement,
		);
		expect(resolveFocus(rootOf([again]), { kind: "run" })).toBe(
			again as unknown as HTMLElement,
		);
		expect(resolveFocus(rootOf([run, again]), { kind: "runAgain" })).toBe(
			again as unknown as HTMLElement,
		);
		expect(resolveFocus(rootOf([run]), { kind: "copy" })).toBe(
			run as unknown as HTMLElement,
		);
	});

	test("a field that is gone has no stand-in", () => {
		expect(
			resolveFocus(rootOf([node("run")]), field("vendor_name")),
		).toBeNull();
	});
});

describe("applyFocusRequest", () => {
	test("focuses the element; a field request can select its value", () => {
		const vendor = node("field:vendor_name");
		const root = rootOf([vendor]);
		expect(applyFocusRequest(root, request(field("vendor_name")))).toBe(true);
		expect([vendor.focused, vendor.selected]).toEqual([1, 0]);
		applyFocusRequest(root, request(field("vendor_name", { select: true })));
		expect([vendor.focused, vendor.selected]).toEqual([2, 1]);
	});

	test("a scroll-only request on a phone brings the field into view without focus", () => {
		const date = node("field:invoice_date");
		const root = rootOf([date]);
		expect(
			applyFocusRequest(
				root,
				request(field("invoice_date", { scrollOnly: true })),
			),
		).toBe(true);
		expect([date.focused, date.scrolled]).toEqual([0, 1]);
		expect(root.focused).toBe(0);
	});

	test("the same request on a split box gives the field the cursor", () => {
		const date = node("field:invoice_date");
		const root = rootOf([date]);
		const after = request(
			field("invoice_date", { select: true, scrollOnly: true }),
		);
		expect(applyFocusRequest(root, after, true)).toBe(true);
		expect([date.focused, date.selected, date.scrolled]).toEqual([1, 1, 0]);
	});

	test("a field not asked to select gets the caret after its value", () => {
		const ranges: [number, number][] = [];
		const typed = {
			...node("field:invoice_date"),
			value: "30",
			setSelectionRange(start: number, end: number) {
				ranges.push([start, end]);
			},
		};
		const root = rootOf([typed]);
		applyFocusRequest(root, request(field("invoice_date")));
		expect(typed.focused).toBe(1);
		expect(ranges).toEqual([[2, 2]]);
	});

	test("an element that is not there leaves focus on the interface, never on the page", () => {
		const root = rootOf([node("run")]);
		expect(applyFocusRequest(root, request(field("vendor_name")))).toBe(false);
		expect(root.focused).toBe(1);
	});

	test("non-text elements are not selected", () => {
		const button = { ...node("run"), select: undefined };
		const root = rootOf([button as unknown as FakeNode]);
		expect(applyFocusRequest(root, request({ kind: "run" }))).toBe(true);
		expect(button.focused).toBe(1);
	});
});

const touch: WorkbenchLayout = { ...PHONE_LAYOUT };

function reopened(): FormSessionState {
	return fixture("reopen");
}

describe("autofocusKey (S6)", () => {
	test("a repeat user opening the page lands in the first empty required field", () => {
		expect(autofocusKey(reopened(), DESKTOP_LAYOUT)).toBe("invoice_file");
	});

	test("when nothing required is empty it lands in the first field", () => {
		const filled = fixture("done");
		expect(filled.runs.length).toBeGreaterThanOrEqual(2);
		expect(autofocusKey(filled, DESKTOP_LAYOUT)).toBe("invoice_file");
	});

	test("never on a first visit: fewer than two runs on this device", () => {
		expect(autofocusKey(fixture("idle"), DESKTOP_LAYOUT)).toBeNull();
		const state = reopened();
		expect(
			autofocusKey({ ...state, runs: state.runs.slice(0, 1) }, DESKTOP_LAYOUT),
		).toBeNull();
		expect(
			autofocusKey({ ...state, runs: state.runs.slice(0, 2) }, DESKTOP_LAYOUT),
		).toBe("invoice_file");
	});

	test("never in a tile", () => {
		const state = reopened();
		const tile: FormSessionState = {
			...state,
			form: {
				...state.form,
				host: { ...state.form.host, presentation: "tile" },
			},
		};
		expect(autofocusKey(tile, DESKTOP_LAYOUT)).toBeNull();
	});

	test("never on a touch screen, in a narrow box, or without a fine pointer", () => {
		const state = reopened();
		expect(autofocusKey(state, touch)).toBeNull();
		expect(autofocusKey(state, { ...DESKTOP_LAYOUT, split: false })).toBeNull();
		expect(autofocusKey(state, { ...DESKTOP_LAYOUT, touch: true })).toBeNull();
		expect(
			autofocusKey(state, { ...DESKTOP_LAYOUT, finePointer: false }),
		).toBeNull();
		expect(autofocusKey(state, null)).toBeNull();
	});

	test("never in a form without fields", () => {
		const state = fixture("none-done");
		expect(
			autofocusKey({ ...state, runs: fixture("done").runs }, DESKTOP_LAYOUT),
		).toBeNull();
	});

	test("waits for the device memory and for the Inputs tab", () => {
		const state = reopened();
		expect(
			autofocusKey(
				{ ...state, memory: { ...state.memory, loaded: false } },
				DESKTOP_LAYOUT,
			),
		).toBeNull();
		expect(
			autofocusKey(
				{ ...state, rail: { ...state.rail, tab: "runs" } },
				DESKTOP_LAYOUT,
			),
		).toBeNull();
	});

	test("skips a file field this page cannot fill", () => {
		const hosted = fixture("hosted-files");
		const state: FormSessionState = { ...hosted, runs: fixture("reopen").runs };
		expect(autofocusKey(state, DESKTOP_LAYOUT)).toBe("vendor_name");
	});
});

describe("QUIET_FOCUS_CSS", () => {
	test("hides outlines and rings under the root while the attribute is on, with no child combinator", () => {
		expect(QUIET_FOCUS_CSS).toStartWith(
			"[data-fw-root][data-fw-quiet-focus] :is(",
		);
		for (const selector of [
			":focus",
			":focus-visible",
			":focus-within",
			":has(:focus-visible)",
		])
			expect(QUIET_FOCUS_CSS).toContain(selector);
		expect(QUIET_FOCUS_CSS).toContain("outline:none !important");
		expect(QUIET_FOCUS_CSS).toContain("box-shadow:none !important");
		expect(QUIET_FOCUS_CSS).not.toContain(">");
	});

	test("keeps the label line as the hover alone sets it: Reset hidden, Optional shown, no quotes", () => {
		for (const scope of ["field", "prop"]) {
			const resting = `[data-fw-root][data-fw-quiet-focus] .group\\/${scope}:not(:hover)`;
			expect(QUIET_FOCUS_CSS).toContain(`${resting} [data-fw-reveal=${scope}]`);
			expect(QUIET_FOCUS_CSS).toContain(
				`${resting} [data-fw-reveal-hide=${scope}]`,
			);
		}
		expect(QUIET_FOCUS_CSS).toContain(
			"{opacity:0 !important;pointer-events:none !important}",
		);
		expect(QUIET_FOCUS_CSS).toContain("{visibility:visible !important}");
		expect(QUIET_FOCUS_CSS).not.toMatch(/["'&<]/);
	});
});

describe("SELECTION_CSS", () => {
	test("paints selected text neutral in the form and its dialogs only, text colour set, safe for SSR", () => {
		expect(SELECTION_CSS).toStartWith(
			"[data-fw-root] ::selection,[data-fw-modal] ::selection{",
		);
		expect(SELECTION_CSS).toContain(
			"background:color-mix(in oklab,var(--foreground) 18%,transparent)",
		);
		expect(SELECTION_CSS).toContain("color:var(--foreground)");
		expect(SELECTION_CSS).not.toContain("primary");
		expect(SELECTION_CSS).not.toMatch(/["'&<>]/);
	});
});
