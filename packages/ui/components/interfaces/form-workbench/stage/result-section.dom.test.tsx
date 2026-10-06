import {
	afterAll,
	afterEach,
	describe,
	expect,
	setDefaultTimeout,
	test,
} from "bun:test";
import type { ResultFormat } from "../run/result-view";
import {
	allByRole,
	byRole,
	byText,
	click,
	mountWorkbench,
	queryByRole,
} from "../testing/dom";
import { FIXTURE_RESULT, FIXTURE_RESULT_SMALL } from "../testing/fixtures";
import { DESKTOP_LAYOUT, PHONE_LAYOUT } from "../testing/layouts";
import { installStageDom } from "./dom-setup";

setDefaultTimeout(60_000);

const dom = installStageDom();
const { ResultSection } = await import("./sections/result-section");

afterEach(dom.cleanup);
afterAll(dom.restore);

const FORMAT: ResultFormat = {
	locale: "en-GB",
	words: { yes: "Yes", no: "No", none: "None", emptyText: "Empty text" },
};

const mountResult = (value: unknown, touch = false) =>
	mountWorkbench(
		<ResultSection value={value} format={FORMAT} touch={touch} />,
		{
			layout: touch ? PHONE_LAYOUT : DESKTOP_LAYOUT,
		},
	);

describe("ResultSection", () => {
	test("an object reads as labelled rows, with a tone chip for a status", async () => {
		await mountResult(FIXTURE_RESULT_SMALL);
		expect(byText("Return number")).toBeTruthy();
		expect(byText("RET-20931")).toBeTruthy();
		const status = byText("Accepted");
		expect(status.closest("span")?.className).toContain("border-good-line");
		expect(byText("Sent by e-mail")).toBeTruthy();
		expect(queryByRole("table")).toBeNull();
	});

	test("one small card keeps the section narrow", async () => {
		const { container } = await mountResult(FIXTURE_RESULT_SMALL);
		const section = container.querySelector("[data-sec='result']");
		expect(section?.className).toContain("max-w-104");
	});

	test("a richer result shows its groups and its table of line items", async () => {
		const { container } = await mountResult(FIXTURE_RESULT);
		const section = container.querySelector("[data-sec='result']");
		expect(section?.className).not.toContain("max-w-104");
		const tables = allByRole("table");
		expect(tables.length).toBeGreaterThan(0);
		const heads = allByRole(
			"columnheader",
			undefined,
			tables[0] as HTMLElement,
		).map((head) => head.textContent);
		expect(heads.length).toBeGreaterThan(2);
		expect(container.textContent).toContain("Nordwind Logistik GmbH");
		expect(container.textContent).not.toContain('"invoice_number"');
	});

	test("Raw swaps the rows for the JSON and back", async () => {
		const { container } = await mountResult(FIXTURE_RESULT_SMALL);
		const toggle = byRole("button", "Raw");
		expect(toggle.getAttribute("aria-pressed")).toBe("false");
		await click(toggle);
		expect(toggle.getAttribute("aria-pressed")).toBe("true");
		const raw = container.querySelector("pre");
		expect(raw?.textContent).toContain('"return_number": "RET-20931"');
		await click(toggle);
		expect(container.querySelector("pre")).toBeNull();
		expect(byText("Return number")).toBeTruthy();
	});

	test("a lone value is one small row, not a JSON dump", async () => {
		const { container } = await mountResult("done");
		expect(container.querySelector("pre")).toBeNull();
		expect(byText("Done")).toBeTruthy();
	});

	test("false, zero and an empty string are results too", async () => {
		const flag = await mountResult(false);
		expect(byText("No")).toBeTruthy();
		await flag.unmount();
		const zero = await mountResult(0);
		expect(byText("0")).toBeTruthy();
		await zero.unmount();
		const empty = await mountResult("");
		expect(byText("Empty text")).toBeTruthy();
		await empty.unmount();
	});

	test("on a touch screen the Raw toggle is a 44 px target", async () => {
		await mountResult(FIXTURE_RESULT_SMALL, true);
		expect(byRole("button", "Raw").className).toContain("h-11");
	});
});
