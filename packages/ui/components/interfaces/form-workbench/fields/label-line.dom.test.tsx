import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { QUIET_FOCUS_ATTR } from "../contracts";
import { installWorkbenchDom } from "../testing/dom";
import { DESKTOP_LAYOUT, PHONE_LAYOUT } from "../testing/layouts";

/*
 * The label line under the shell's quiet focus (S6): the cursor rests in the first field on open, and
 * only a hover may reveal "Clear" / "Reset" or hide "Optional" then, as on the canvas.
 */

const dom = installWorkbenchDom();
const { LiveField, liveStore } = await import("./field-testing");
const { QUIET_FOCUS_CSS } = await import("../shell/focus");

afterEach(dom.cleanup);
afterAll(dom.restore);

/** The selectors of one quiet rule, by what its declarations do. */
function quietSelectors(declarations: string) {
	const rule = QUIET_FOCUS_CSS.split("}").find((part) =>
		part.includes(`{${declarations}`),
	);
	if (!rule) throw new Error(`no quiet rule with ${declarations}`);
	return rule.slice(0, rule.indexOf("{")).split(",");
}

async function renderField(name: string, layout = DESKTOP_LAYOUT) {
	const store = liveStore("done", { layout });
	const view = await dom.render(
		<div data-fw-root="" {...{ [QUIET_FOCUS_ATTR]: "" }}>
			<LiveField store={store} name={name} />
		</div>,
	);
	return view.container;
}

const matchesAny = (element: Element | null, selectors: readonly string[]) =>
	element !== null && selectors.some((selector) => element.matches(selector));

describe("the label line while focus is quiet", () => {
	test("Clear is a hover reveal of its field, and the quiet rule keeps it hidden", async () => {
		const container = await renderField("invoice_file");
		const clear = container.querySelector('[aria-label="Clear Invoice"]');
		expect(clear?.getAttribute("data-fw-reveal")).toBe("field");
		expect(matchesAny(clear, quietSelectors("opacity:0 !important"))).toBe(
			true,
		);
	});

	test("Optional, hidden by a focus reveal, is shown again by the quiet rule", async () => {
		const container = await renderField("supporting_documents");
		const optional = Array.from(container.querySelectorAll("span")).find(
			(span) => span.textContent === "Optional",
		);
		expect(optional?.getAttribute("data-fw-reveal-hide")).toBe("field");
		expect(
			matchesAny(
				optional ?? null,
				quietSelectors("visibility:visible !important"),
			),
		).toBe(true);
	});

	test("a touch screen has no hover reveal: Clear stays and nothing is marked", async () => {
		const container = await renderField("invoice_file", PHONE_LAYOUT);
		const clear = container.querySelector('[aria-label="Clear Invoice"]');
		expect(clear).not.toBeNull();
		expect(clear?.hasAttribute("data-fw-reveal")).toBe(false);
		expect(container.querySelector("[data-fw-reveal-hide]")).toBeNull();
	});

	test("without the quiet attribute the rules match nothing", async () => {
		const store = liveStore("done");
		const view = await dom.render(
			<div data-fw-root="">
				<LiveField store={store} name="invoice_file" />
			</div>,
		);
		const clear = view.container.querySelector('[aria-label="Clear Invoice"]');
		expect(matchesAny(clear, quietSelectors("opacity:0 !important"))).toBe(
			false,
		);
	});
});
