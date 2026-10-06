import {
	afterAll,
	afterEach,
	describe,
	expect,
	setSystemTime,
	test,
} from "bun:test";
import { installWorkbenchDom, mountWorkbench } from "../testing/dom";
import { fakeView } from "../testing/fake-actions";
import { fixture, fixtureClock } from "../testing/fixtures";
import { DESKTOP_LAYOUT } from "../testing/layouts";

const dom = installWorkbenchDom();
const { RunsList } = await import("./runs-list");
const { clearScrollMemory } = await import("./use-scroll-memory");

afterEach(async () => {
	clearScrollMemory();
	setSystemTime();
	await dom.cleanup();
});
afterAll(() => dom.restore());

async function show() {
	setSystemTime(new Date(fixtureClock("runs").now));
	const view = fakeView(fixture("runs"), { layout: DESKTOP_LAYOUT });
	await mountWorkbench(
		<RunsList
			state={view.props.state}
			actions={view.props.actions}
			layout={view.props.layout}
		/>,
		{ layout: DESKTOP_LAYOUT },
	);
}

const row = (id: string) => {
	const found = document.querySelector<HTMLElement>(`[data-run-id="${id}"]`);
	if (!found) throw new Error(`no row for ${id}`);
	return found;
};

/** The change line and the preview of a row: the spans under the first line. */
const linesOf = (id: string) =>
	Array.from(
		row(id).querySelectorAll<HTMLElement>(":scope > span.col-start-2"),
	);

describe("a row of the Runs list", () => {
	test("an identifier stays in one piece: its hyphens are non-breaking, the tooltip keeps the text", async () => {
		await show();
		const [changes] = linesOf("run-13");
		expect(changes?.textContent).toContain(
			"invoice\u{2011}RE\u{2011}2026\u{2011}0917.pdf",
		);
		expect(changes?.textContent).not.toMatch(/[A-Za-z0-9]-[A-Za-z0-9]/);
		expect(changes?.getAttribute("title")).toContain(
			"invoice-RE-2026-0917.pdf",
		);
	});

	test("the preview of a finished run keeps its identifiers whole as well", async () => {
		await show();
		const previews = ["run-11", "run-9", "run-8"].flatMap((id) =>
			linesOf(id).map((line) => line.textContent ?? ""),
		);
		expect(previews.some((text) => text.includes("PO\u{2011}"))).toBe(true);
		for (const text of previews) expect(text).not.toMatch(/PO-\d/);
	});

	test("the time column tightens its word spacing, as the canvas does", async () => {
		await show();
		const when = row("run-14").querySelector<HTMLElement>("span.font-mono");
		expect(when?.textContent).toMatch(/14:02/);
		expect(when?.className).toContain("[word-spacing:-0.25em]");
	});

	test("a failed run reads 'Failed at “step”.' and a stopped one 'Stopped at “step”.'", async () => {
		await show();
		const failed = linesOf("run-13").at(-1)?.textContent;
		expect(failed).toBe("Failed at “Run OCR”.");
		const stopped = linesOf("run-10").at(-1)?.textContent;
		expect(stopped).toMatch(/^Stopped at “[^”]+”\.$/);
	});
});
