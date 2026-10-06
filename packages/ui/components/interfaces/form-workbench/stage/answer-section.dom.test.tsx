import {
	afterAll,
	afterEach,
	describe,
	expect,
	setDefaultTimeout,
	test,
} from "bun:test";
import { mountWorkbench } from "../testing/dom";
import { DESKTOP_LAYOUT } from "../testing/layouts";
import { NUMERIC_ATTR } from "./answer-table";
import { installStageDom } from "./dom-setup";

setDefaultTimeout(60_000);

const dom = installStageDom();
const { AnswerSection } = await import("./sections/answer-section");
const { CARET_MARKER } = await import("./sections/answer-style");

afterEach(dom.cleanup);
afterAll(dom.restore);

const ANSWER = [
	"## Invoice RE-2026-0917 extracted",
	"",
	"All 14 pages were read.",
	"",
	"| # | Description | Qty | Unit price | Net |",
	"|---|---|---|---|---|",
	"| 1 | Pallet transport Hamburg–Munich | 12 | € 410.00 | € 4,920.00 |",
	"| 2 | Cold-chain surcharge | 12 | € 85.00 | € 1,020.00 |",
	"",
	"### Needs your attention",
].join("\n");

const shownTable = () => {
	const table = Array.from(document.querySelectorAll("table")).find(
		(item) => item.getAttribute("aria-hidden") !== "true",
	);
	if (!table) throw new Error("no table in the answer");
	return table;
};

const numericOf = (cells: readonly Element[]) =>
	cells.map((cell) => cell.hasAttribute(NUMERIC_ATTR));

async function mount(live: boolean, answer = ANSWER) {
	return mountWorkbench(
		<AnswerSection answer={answer} live={live} incomplete={false} />,
		{ layout: DESKTOP_LAYOUT },
	);
}

describe("the answer's table", () => {
	test("numeric columns are marked, header and body; the running index and text are not", async () => {
		await mount(false);
		const table = shownTable();
		expect(numericOf(Array.from(table.querySelectorAll("thead th")))).toEqual([
			false,
			false,
			true,
			true,
			true,
		]);
		for (const row of Array.from(table.querySelectorAll("tbody tr")))
			expect(numericOf(Array.from(row.querySelectorAll("td")))).toEqual([
				false,
				false,
				true,
				true,
				true,
			]);
	});

	test("a streamed table is marked once its rows arrive", async () => {
		await mount(true);
		const head = Array.from(shownTable().querySelectorAll("thead th"));
		expect(numericOf(head)).toEqual([false, false, true, true, true]);
	});

	test("the answer's own table look is scoped to the workbench answer", async () => {
		await mount(false);
		const prose = document.querySelector("[data-fl-chat-prose]");
		const classes = prose?.className ?? "";
		for (const rule of [
			"[&_.slate-table>div>div:first-child]:hidden",
			"[&_.slate-table_th_svg]:hidden",
			"[&_.slate-table_td]:border-x-0",
			"[&_.slate-table_td[data-fw-num]_*]:font-mono",
			"[&_.slate-p]:py-0",
			"[&_.slate-p]:mb-3.5",
			"[&_.slate-editor_h3.slate-h3]:mt-6.5",
		])
			expect(classes).toContain(rule);
	});

	test("the answer's type follows the canvas: h2 at 26/32, bold at 600, muted small list dots", async () => {
		await mount(false);
		const classes =
			document.querySelector("[data-fl-chat-prose]")?.className ?? "";
		for (const rule of [
			"[&_.slate-editor_h2.slate-h2]:text-[26px]/[32px]",
			"[&_.slate-editor_h2.slate-h2]:tracking-[-0.01em]!",
			"[&_.slate-editor_h3.slate-h3]:tracking-normal!",
			"[&_strong]:font-semibold",
			"[&_li]:marker:text-muted-foreground",
			"[&_[data-slate-list-style-type=disc]_li]:marker:text-[0.8em]",
		])
			expect(classes).toContain(rule);
		expect(classes).not.toContain("text-pretty");
	});
});

describe("dates and amounts in the answer", () => {
	const NB = "\u{a0}";
	const TEXT = [
		"**Gross:** € 12,483.10 · VAT 19 % · **Due:** 17 Oct 2026",
		"",
		"`17 Oct 2026` stays as written.",
	].join("\n");

	test("are tied with no-break spaces on screen, while the answer itself is left as written", async () => {
		await mount(false, TEXT);
		const shown = document.querySelector("[data-fl-chat-prose]")?.textContent;
		expect(shown).toContain(`Due:${NB}17${NB}Oct${NB}2026`);
		expect(shown).toContain(`€${NB}12,483.10`);
		expect(shown).toContain(`19${NB}%`);
		expect(shown).toContain(`17 Oct 2026 stays as${NB}written.`);
		expect(TEXT).not.toContain(NB);
	});

	test("are tied while the answer streams as well", async () => {
		await mount(true, TEXT);
		const shown = document.querySelector("[data-fl-chat-prose]")?.textContent;
		expect(shown).toContain(`Due:${NB}17${NB}Oct${NB}2026`);
	});
});

describe("the follow marker while the answer streams", () => {
	test("sits on the caret's line: lifted by the last block's margin and the line under the caret", async () => {
		await mount(true);
		const tail = document.querySelector("[data-fw-tail]");
		expect(tail?.className).toBe(CARET_MARKER);
		expect(CARET_MARKER).toContain("-top-[21px]");
	});

	test("sits after the text without adding a gap to the section", async () => {
		await mount(true);
		const section = document.querySelector('[data-sec="answer"]');
		const tail = section?.querySelector("[data-fw-tail]");
		expect(tail).toBeTruthy();
		expect(tail?.parentElement).not.toBe(section);
		expect(
			tail?.previousElementSibling?.hasAttribute("data-fl-chat-prose"),
		).toBe(true);
	});

	test("is gone once the run ended", async () => {
		await mount(false);
		expect(document.querySelector("[data-fw-tail]")).toBeNull();
	});
});
