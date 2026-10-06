import { afterAll, afterEach, describe, expect, mock, test } from "bun:test";
import { dirname } from "node:path";
import {
	advance,
	byRole,
	byText,
	click,
	fire,
	installWorkbenchDom,
	mountWorkbench,
	queryByRole,
	settle,
	typeInto,
} from "../testing/dom";
import { PHONE_LAYOUT } from "../testing/layouts";
import { blurOff, focusOn, press } from "./field-events";

/*
 * react-day-picker ships its own copy of react (a second 19.x under its node_modules), which breaks hooks under bun.
 * Bundlers dedupe it; here the nested copy is pointed at the root one.
 */
const rootReact = Bun.resolveSync("react", process.cwd());
const nestedReact = Bun.resolveSync(
	"react",
	dirname(Bun.resolveSync("react-day-picker", process.cwd())),
);
if (nestedReact !== rootReact)
	mock.module(nestedReact, () => require(rootReact));

const dom = installWorkbenchDom();
const { LiveField, liveStore } = await import("./field-testing");

afterEach(dom.cleanup);
afterAll(dom.restore);

type Store = ReturnType<typeof liveStore>;

async function mount(
	fixtureName: Parameters<typeof liveStore>[0],
	options: Parameters<typeof liveStore>[1] = {},
) {
	const store: Store = liveStore(fixtureName, options);
	const view = await mountWorkbench(
		<LiveField store={store} name="invoice_date" />,
		{ layout: store.layout },
	);
	return { store, view };
}

const input = () => byRole("textbox", "Invoice date") as HTMLInputElement;
const reading = () => document.querySelector("output") as HTMLElement;

const withAnchor = (iso: string) => (state: ReturnType<Store["snapshot"]>) => ({
	...state,
	rail: { ...state.rail, dateAnchors: { invoice_date: iso } },
});

describe("typed date (spec M4)", () => {
	test("an empty field says Pick a date; the calendar button is no Tab stop", async () => {
		await mount("idle");
		expect(input().getAttribute("placeholder")).toBe("Pick a date");
		expect(input().value).toBe("");
		const button = byRole("button", "Choose invoice date from a calendar");
		expect(button.getAttribute("tabindex")).toBe("-1");
		expect(input().getAttribute("aria-keyshortcuts")).toBe("Alt+ArrowDown");
		expect(input().getAttribute("data-fw-focus")).toBe("field:invoice_date");
	});

	test("the committed day shows as 17 Sep 2026", async () => {
		await mount("idle", {
			change: (state) => ({
				...state,
				rail: {
					...state.rail,
					values: { ...state.rail.values, invoice_date: "2026-09-17" },
				},
			}),
		});
		expect(input().value).toBe("17 Sep 2026");
	});

	test("typing 18 reads as Fri 18 Sep 2026 from the last date, until it is committed", async () => {
		const { store } = await mount("reopen");
		await typeInto(input(), "18");
		expect(store.fake.argsOf("setText")).toEqual([["invoice_date", "18"]]);
		expect(reading().textContent).toBe("Reads as Fri 18 Sep 2026");
		expect(input().value).toBe("18");
	});

	test("a day more than 14 days before the last date reads with the warning", async () => {
		await mount("reopen", { change: withAnchor("2026-09-30") });
		await typeInto(input(), "1");
		expect(reading().textContent).toBe(
			"Reads as Tue 1 Sep 2026 · 29 days before the last date",
		);
		expect(reading().className).toContain("text-warning");
		expect(reading().querySelector("svg")).not.toBeNull();
	});

	test("2/10 reads in the viewer's order", async () => {
		await mount("reopen");
		await typeInto(input(), "2/10");
		expect(reading().textContent).toBe("Reads as Fri 2 Oct 2026");
	});

	test("text that cannot be read has no reading", async () => {
		await mount("reopen");
		await typeInto(input(), "31/31");
		expect(document.querySelector("output")).toBeNull();
	});

	test("Tab or leaving the input validates, and the reducer commits typed text with it", async () => {
		const { store } = await mount("reopen");
		const box = input();
		await focusOn(box);
		await typeInto(box, "18");
		await blurOff(box);
		expect(store.fake.argsOf("blurField")).toEqual([["invoice_date"]]);
		expect(store.fake.argsOf("commitText")).toEqual([]);
		expect(store.state.rail.values.invoice_date).toBe("2026-09-18");
		expect(input().value).toBe("18 Sep 2026");
		await focusOn(input());
		await blurOff(input());
		expect(store.fake.argsOf("blurField")).toHaveLength(2);
	});

	test("going to the calendar button commits the text and holds the validation back", async () => {
		const { store } = await mount("idle");
		const box = input();
		await focusOn(box);
		await typeInto(box, "18");
		const button = byRole("button", "Choose invoice date from a calendar");
		await fire(button, new window.Event("pointerdown", { bubbles: true }));
		await blurOff(box);
		expect(store.fake.argsOf("commitText")).toEqual([["invoice_date"]]);
		expect(store.fake.argsOf("blurField")).toEqual([]);
	});

	test("an open calendar holds the validation back too", async () => {
		const { store } = await mount("idle");
		await focusOn(input());
		await press(input(), "ArrowDown", { altKey: true });
		await blurOff(input());
		expect(store.fake.argsOf("blurField")).toEqual([]);
	});

	test("↵ asks the reducer to commit and move on; an IME's ↵ and a held ↵ do not", async () => {
		const { store } = await mount("reopen");
		await typeInto(input(), "18");
		await press(input(), "Enter", { isComposing: true });
		await press(input(), "Enter", { repeat: true });
		expect(store.fake.argsOf("enter")).toEqual([]);
		const event = await press(input(), "Enter");
		expect(event.defaultPrevented).toBe(true);
		expect(store.fake.argsOf("enter")).toEqual([["invoice_date"]]);
	});

	test("an unreadable date shows the sentence in the viewer's order", async () => {
		await mount("idle", {
			change: (state) => ({
				...state,
				rail: { ...state.rail, problems: { invoice_date: { code: "date" } } },
			}),
		});
		expect(byRole("alert").textContent).toBe(
			"Enter a date such as 21/9 or 21/9/2026.",
		);
		expect(input().getAttribute("aria-invalid")).toBe("true");
	});

	test("a required empty date asks to choose one", async () => {
		await mount("idle", {
			change: (state) => ({
				...state,
				rail: {
					...state.rail,
					problems: { invoice_date: { code: "required" } },
				},
			}),
		});
		expect(byRole("alert").textContent).toBe("Choose a date.");
	});
});

describe("the calendar (spec M4)", () => {
	test("⌥↓ opens it at the last date's month with the cursor on that day", async () => {
		await mount("reopen");
		const event = await press(input(), "ArrowDown", { altKey: true });
		expect(event.defaultPrevented).toBe(true);
		const dialog = byRole("dialog", "Choose a date");
		expect(dialog.textContent).toContain("September 2026");
		await settle();
		expect(dom.document.activeElement?.textContent).toBe("17");
	});

	test("typed text that reads as a day is where the cursor starts", async () => {
		await mount("reopen");
		await typeInto(input(), "18");
		await press(input(), "ArrowDown", { altKey: true });
		await settle();
		expect(dom.document.activeElement?.textContent).toBe("18");
	});

	test("an empty field of a first visit opens at today's month", async () => {
		await mount("idle");
		await press(input(), "ArrowDown", { altKey: true });
		expect(byRole("dialog", "Choose a date").textContent).toContain(
			"October 2026",
		);
	});

	test("picking a day sets it, closes the calendar and returns the cursor to the input", async () => {
		const { store } = await mount("reopen");
		await press(input(), "ArrowDown", { altKey: true });
		await click(byText("18", byRole("dialog", "Choose a date")));
		expect(store.fake.argsOf("setValue")).toEqual([
			["invoice_date", "2026-09-18"],
		]);
		await advance(20);
		expect(queryByRole("dialog", "Choose a date")).toBeNull();
		expect(dom.document.activeElement).toBe(input());
	});

	test("the calendar button opens it too, and the chosen day is selected", async () => {
		await mount("idle", {
			change: (state) => ({
				...state,
				rail: {
					...state.rail,
					values: { ...state.rail.values, invoice_date: "2026-09-17" },
				},
			}),
		});
		await click(byRole("button", "Choose invoice date from a calendar"));
		const dialog = byRole("dialog", "Choose a date");
		expect(dialog.textContent).toContain("September 2026");
		const selected = dialog.querySelector("[aria-selected=true]");
		expect(selected?.textContent).toBe("17");
	});

	test("Esc closes it", async () => {
		await mount("reopen");
		await press(input(), "ArrowDown", { altKey: true });
		await press(byRole("dialog", "Choose a date"), "Escape");
		await advance(20);
		expect(queryByRole("dialog", "Choose a date")).toBeNull();
	});
});

describe("touch screens keep the native date input", () => {
	test("a date input at 44 px with the committed day", async () => {
		const { store } = await mount("idle", {
			layout: PHONE_LAYOUT,
			change: (state) => ({
				...state,
				rail: {
					...state.rail,
					values: { ...state.rail.values, invoice_date: "2026-09-17" },
				},
			}),
		});
		const native = document.querySelector(
			"input[type=date]",
		) as HTMLInputElement;
		expect(native.value).toBe("2026-09-17");
		expect(native.className).toContain("h-11");
		expect(
			queryByRole("button", "Choose invoice date from a calendar"),
		).toBeNull();
		await typeInto(native, "2026-09-18");
		expect(store.fake.argsOf("setValue")).toEqual([
			["invoice_date", "2026-09-18"],
		]);
	});
});
