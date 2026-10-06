import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type {
	FormSessionState,
	WorkbenchField,
	WorkbenchLayout,
} from "../contracts";
import { FIELD_KEY_SEPARATOR } from "../contracts";
import {
	allByRole,
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
import { focusOn, press } from "./field-events";

const dom = installWorkbenchDom();
const { LiveField, liveStore, patchField, patchRail } = await import(
	"./field-testing"
);

afterEach(dom.cleanup);
afterAll(dom.restore);

type Store = ReturnType<typeof liveStore>;
type Change = (state: FormSessionState) => FormSessionState;

async function mount(
	fixtureName: Parameters<typeof liveStore>[0],
	name: string,
	options: { change?: Change; layout?: WorkbenchLayout } = {},
) {
	const store: Store = liveStore(fixtureName, options);
	await mountWorkbench(<LiveField store={store} name={name} />, {
		layout: store.layout,
	});
	return store;
}

const chain =
	(...changes: Change[]): Change =>
	(state) =>
		changes.reduce((next, change) => change(next), state);

async function paste(element: Element, text: string) {
	const event = new window.Event("paste", { bubbles: true, cancelable: true });
	Object.defineProperty(event, "clipboardData", {
		value: { getData: () => text },
	});
	await fire(element, event);
	return event;
}

/** The entry of a list: a combobox once the form has recent values to offer, else a textbox. */
const entryOf = () =>
	document.querySelector("fieldset input") as HTMLInputElement;

const key = (group: string, prop: string) =>
	`${group}${FIELD_KEY_SEPARATOR}${prop}`;

describe("the label line (spec M1)", () => {
	test("a changed value has the dot and a Reset that is no Tab stop", async () => {
		const store = await mount("done", "expected_total");
		expect(byRole("img", "Changed from its default").className).toContain(
			"rounded-full",
		);
		const reset = byRole("button", "Reset Expected total");
		expect(reset.getAttribute("tabindex")).toBe("-1");
		await click(reset);
		expect(store.fake.argsOf("resetField")).toEqual([["expected_total"]]);
	});

	test("an unchanged field has neither; Optional marks the optional ones only", async () => {
		await mount("done", "max_pages");
		expect(queryByRole("img", "Changed from its default")).toBeNull();
		expect(queryByRole("button", "Reset Max pages")).toBeNull();
		expect(document.body.textContent).toContain("Optional");
		document.body.innerHTML = "";
	});

	test("a required field without a default says Clear once it holds a value", async () => {
		await mount("done", "vendor_name", {
			change: patchRail({ values: { vendor_name: "Acme" } }),
		});
		byRole("button", "Clear Vendor");
		expect(document.body.textContent).not.toContain("Optional");
	});

	test("⇧⌘⌫ resets a field that differs and leaves one that does not", async () => {
		const store = await mount("done", "expected_total");
		const box = byRole("textbox", "Expected total");
		expect(box.getAttribute("aria-keyshortcuts")).toBe("Meta+Shift+Backspace");
		const event = await press(box, "Backspace", {
			shiftKey: true,
			metaKey: true,
		});
		expect(event.defaultPrevented).toBe(true);
		expect(store.fake.argsOf("resetField")).toEqual([["expected_total"]]);
		await press(box, "Backspace", { shiftKey: true });
		await press(box, "Backspace", { metaKey: true });
		expect(store.fake.argsOf("resetField")).toHaveLength(1);
	});

	test("the marker opens the Per run popover on its field; touch shows it as text", async () => {
		const store = await mount("series", "invoice_date");
		const marker = byRole("button", "Invoice date is per run. Change");
		expect(marker.getAttribute("tabindex")).toBe("-1");
		expect(marker.getAttribute("title")).toBe(
			"Goes back to its starting value after each run. Click to change.",
		);
		await click(marker);
		expect(store.fake.argsOf("openOverlay")).toEqual([
			[{ id: "afterRun", focusName: "invoice_date" }],
		]);
		document.body.innerHTML = "";
	});

	test("a field that waits for its next file reads Next file", async () => {
		await mount("series", "invoice_file");
		const marker = byRole(
			"button",
			"Invoice takes the next file after each run. Change",
		);
		expect(marker.textContent).toBe("Next file");
	});

	test("on a touch screen the marker is plain text", async () => {
		await mount("series", "invoice_date", { layout: PHONE_LAYOUT });
		expect(queryByRole("button", "Invoice date is per run. Change")).toBeNull();
		byText("Per run");
	});
});

describe("a number (spec S3)", () => {
	test("↑ ↓ change by a step, ⇧ by ten, typed decimals are kept", async () => {
		const store = await mount("done", "max_pages");
		const box = byRole("textbox", "Max pages");
		expect(box.getAttribute("inputmode")).toBe("numeric");
		const up = await press(box, "ArrowUp");
		await press(box, "ArrowUp", { shiftKey: true });
		await press(box, "ArrowDown");
		expect(up.defaultPrevented).toBe(true);
		expect(store.fake.argsOf("setValue")).toEqual([
			["max_pages", "21"],
			["max_pages", "31"],
			["max_pages", "30"],
		]);
	});

	test("an amount with decimals keeps them and ⌥↓ stays with the browser", async () => {
		const store = await mount("done", "expected_total");
		const box = byRole("textbox", "Expected total");
		expect(box.getAttribute("inputmode")).toBe("decimal");
		await press(box, "ArrowUp");
		const alt = await press(box, "ArrowDown", { altKey: true });
		expect(alt.defaultPrevented).toBe(false);
		expect(store.fake.argsOf("setValue")).toEqual([
			["expected_total", "11770.10"],
		]);
	});

	test("a pasted amount is cleaned with the viewer's decimal sign; other text pastes as it is", async () => {
		const store = await mount("done", "expected_total");
		const box = byRole("textbox", "Expected total");
		await focusOn(box);
		const amount = await paste(box, "€ 11.769,10");
		expect(amount.defaultPrevented).toBe(true);
		const plain = await paste(box, "abc");
		expect(plain.defaultPrevented).toBe(false);
		expect(store.fake.argsOf("setValue")).toEqual([
			["expected_total", "11769.10"],
		]);
		await paste(byRole("textbox", "Expected total"), "CHF 1’234.50");
		expect(store.fake.argsOf("setValue")[1]).toEqual([
			"expected_total",
			"1234.50",
		]);
	});

	test("1.500 in a whole-number field is 1500", async () => {
		const store = await mount("done", "max_pages");
		const box = byRole("textbox", "Max pages");
		await focusOn(box);
		await paste(box, "1.500");
		expect(store.fake.argsOf("setValue")).toEqual([["max_pages", "1500"]]);
	});

	test("typing, ↵ and blur", async () => {
		const store = await mount("done", "max_pages");
		const box = byRole("textbox", "Max pages");
		await typeInto(box, "30");
		expect(store.fake.argsOf("setValue")).toEqual([["max_pages", "30"]]);
		const enter = await press(box, "Enter");
		expect(enter.defaultPrevented).toBe(true);
		expect(store.fake.argsOf("enter")).toEqual([["max_pages"]]);
	});

	test("its problems read as sentences", async () => {
		await mount("done", "max_pages", {
			change: patchRail({ problems: { max_pages: { code: "noDecimals" } } }),
		});
		expect(byRole("alert").textContent).toBe(
			"Enter a whole number without decimals.",
		);
		document.body.innerHTML = "";
		await mount("done", "max_pages", {
			change: chain(
				patchField("max_pages", { range: [1, 99] }),
				patchRail({
					problems: { max_pages: { code: "range", min: 1, max: 99 } },
				}),
			),
		});
		expect(byRole("alert").textContent).toBe("Enter a number from 1 to 99.");
	});
});

describe("a switch (spec M2)", () => {
	test("Space toggles through the click, ↵ moves on and never toggles", async () => {
		const store = await mount("done", "run_ocr");
		const row = byRole("switch", "Run OCR");
		expect(row.getAttribute("aria-checked")).toBe("true");
		expect(row.textContent).toBe("On");
		await click(row);
		expect(store.fake.argsOf("setValue")).toEqual([["run_ocr", false]]);
		const enter = await press(byRole("switch", "Run OCR"), "Enter");
		expect(enter.defaultPrevented).toBe(true);
		expect(store.fake.argsOf("enter")).toEqual([["run_ocr"]]);
		expect(store.fake.argsOf("setValue")).toHaveLength(1);
	});

	test("on a touch screen the row carries the label", async () => {
		await mount("done", "run_ocr", { layout: PHONE_LAYOUT });
		const row = byRole("switch");
		expect(row.textContent).toBe("Run OCROn");
		expect(row.className).toContain("min-h-11");
	});
});

describe("a choice (spec M2)", () => {
	const currency = key("payment_terms", "currency");

	test("2–4 short options are a segmented control with a roving Tab stop", async () => {
		await mount("done", currency);
		const group = byRole("radiogroup", "Currency");
		const radios = allByRole("radio");
		expect(radios.map((radio) => radio.textContent)).toEqual([
			"EUR",
			"USD",
			"CHF",
			"GBP",
		]);
		expect(radios.map((radio) => radio.getAttribute("aria-checked"))).toEqual([
			"true",
			"false",
			"false",
			"false",
		]);
		expect(radios.map((radio) => radio.getAttribute("tabindex"))).toEqual([
			"0",
			"-1",
			"-1",
			"-1",
		]);
		expect(radios[0].getAttribute("data-fw-focus")).toBe(`field:${currency}`);
		expect(group.getAttribute("aria-labelledby")).not.toBeNull();
	});

	test("← → pick and wrap, a letter jumps, ↵ moves on", async () => {
		const store = await mount("done", currency);
		const first = allByRole("radio")[0];
		await press(first, "ArrowRight");
		await press(first, "ArrowLeft");
		await press(first, "ArrowLeft");
		await press(first, "c");
		const picked = store.fake.argsOf("setValue").map(([, value]) => value);
		expect(picked).toEqual(["USD", "EUR", "GBP", "CHF"]);
		const enter = await press(first, "Enter");
		expect(enter.defaultPrevented).toBe(true);
		expect(store.fake.argsOf("enter")).toEqual([[currency]]);
	});

	test("more options are a select; its trigger names the value", async () => {
		await mount("large", "locale", {
			change: patchField("locale", {
				kind: "choice",
				options: ["en", "de", "fr", "es", "pt", "ja"],
				defaultValue: "en",
			}),
		});
		const trigger = byRole("combobox", "Locale");
		expect(trigger.getAttribute("data-fw-focus")).toBe("field:locale");
		expect(trigger.getAttribute("aria-labelledby")).not.toBeNull();
	});
});

describe("a list (spec M2)", () => {
	test("chips with their own ×; typing a comma or ↵ adds, ↵ on an empty entry moves on", async () => {
		const store = await mount("done", "cost_centers");
		const entry = byRole("textbox", "Cost centers");
		expect(
			allByRole("button").map((button) => button.getAttribute("aria-label")),
		).toEqual(["Remove 4400", "Remove 4410"]);
		await typeInto(entry, "4420,");
		expect(store.fake.argsOf("setValue")).toEqual([
			["cost_centers", ["4400", "4410", "4420"]],
		]);
		await typeInto(byRole("textbox", "Cost centers"), "4430");
		await press(byRole("textbox", "Cost centers"), "Enter");
		expect(store.fake.argsOf("setValue")[1]).toEqual([
			"cost_centers",
			["4400", "4410", "4420", "4430"],
		]);
		await press(byRole("textbox", "Cost centers"), "Enter");
		expect(store.fake.argsOf("enter")).toEqual([["cost_centers"]]);
	});

	test("← in an empty entry goes into the chips; ⌫ removes the focused chip", async () => {
		const store = await mount("done", "cost_centers");
		const entry = byRole("textbox", "Cost centers");
		await press(entry, "ArrowLeft");
		const chips = Array.from(
			document.querySelectorAll("fieldset span[tabindex='-1']"),
		);
		expect(dom.document.activeElement).toBe(chips[1]);
		await press(chips[1], "ArrowLeft");
		expect(dom.document.activeElement).toBe(chips[0]);
		await press(chips[0], "ArrowRight");
		await press(chips[1], "ArrowRight");
		expect(dom.document.activeElement).toBe(entry);
		await press(entry, "ArrowLeft");
		await press(chips[1], "Backspace");
		await settle();
		expect(store.fake.argsOf("setValue")).toEqual([["cost_centers", ["4400"]]]);
	});

	test("× removes one and the cursor goes back to the entry", async () => {
		const store = await mount("done", "cost_centers");
		await click(byRole("button", "Remove 4400"));
		await settle();
		expect(store.fake.argsOf("setValue")).toEqual([["cost_centers", ["4410"]]]);
		expect(dom.document.activeElement).toBe(entryOf());
	});

	test("a set never takes the same entry twice", async () => {
		const store = await mount("done", "cost_centers", {
			change: patchField("cost_centers", { valueType: "HashSet" }),
		});
		await typeInto(byRole("textbox", "Cost centers"), "4400,");
		expect(store.fake.argsOf("setValue")).toEqual([]);
	});

	test("a list of dates reads each entry in the viewer's order and shows it formatted", async () => {
		const store = await mount("done", "cost_centers", {
			change: chain(
				patchField("cost_centers", { itemKind: "date" }),
				patchRail({ values: { cost_centers: [] } }),
			),
		});
		await typeInto(byRole("textbox", "Cost centers"), "18/10,");
		expect(store.fake.argsOf("setValue")).toEqual([
			["cost_centers", ["2026-10-18"]],
		]);
		await typeInto(byRole("textbox", "Cost centers"), "nope,");
		expect(byRole("alert").textContent).toBe(
			"Enter a date such as 21/9 or 21/9/2026.",
		);
	});

	test("an empty list with recent values points at them", async () => {
		await mount("done", "cost_centers", {
			change: patchRail({ values: { cost_centers: [] } }),
		});
		expect(entryOf().getAttribute("placeholder")).toBe(
			"Type, or press ↓ for recent values",
		);
	});
});

describe("an object with known properties", () => {
	test("a bordered group of nested controls, each with its own focus hook", async () => {
		await mount("done", "payment_terms");
		const group = byRole("group", "Payment terms");
		expect(group.tagName).toBe("FIELDSET");
		expect(group.querySelectorAll("[data-fw-field]")).toHaveLength(4);
		const net = byRole("textbox", "Net days");
		expect(net.getAttribute("data-fw-focus")).toBe(
			`field:${key("payment_terms", "net_days")}`,
		);
		byRole("radiogroup", "Currency");
	});

	test("a property types through its own key", async () => {
		const store = await mount("done", "payment_terms");
		await typeInto(byRole("textbox", "Net days"), "30");
		expect(store.fake.argsOf("setValue")).toEqual([
			[key("payment_terms", "net_days"), "30"],
		]);
	});

	test("on a touch screen the properties are one column", async () => {
		await mount("done", "payment_terms", { layout: PHONE_LAYOUT });
		expect(byRole("group", "Payment terms").className).toContain("grid-cols-1");
	});
});

describe("name and value rows", () => {
	const rows = [{ id: "saved-0", key: "region", value: "eu" }];
	const asPairs = chain(
		patchField("vendor_name", { kind: "pairs", required: false }),
		patchRail({ values: { vendor_name: rows } }),
	);

	test("rows with their inputs, ↵ goes from name to value and then moves on", async () => {
		const store = await mount("idle", "vendor_name", { change: asPairs });
		const name = byRole("textbox", "Name of row 1");
		const value = byRole("textbox", "Value of row 1");
		expect(name.getAttribute("data-fw-focus")).toBe("field:vendor_name");
		await press(name, "Enter");
		expect(dom.document.activeElement).toBe(value);
		await press(value, "Enter");
		expect(store.fake.argsOf("enter")).toEqual([["vendor_name"]]);
	});

	test("editing, adding and removing rows go through the field's value", async () => {
		const store = await mount("idle", "vendor_name", { change: asPairs });
		await typeInto(byRole("textbox", "Value of row 1"), "us");
		expect(store.fake.argsOf("setValue")[0]).toEqual([
			"vendor_name",
			[{ id: "saved-0", key: "region", value: "us" }],
		]);
		await click(byRole("button", "Add row"));
		const added = store.fake.argsOf("setValue")[1][1] as unknown as readonly {
			id: string;
		}[];
		expect(added).toHaveLength(2);
		expect(added[1].id.startsWith("row-")).toBe(true);
		await settle();
		expect(dom.document.activeElement).toBe(byRole("textbox", "Name of row 2"));
		await click(byRole("button", "Remove row 1"));
		expect((store.fake.argsOf("setValue")[2][1] as unknown[]).length).toBe(1);
	});

	test("with no rows the Add row button takes the focus hook", async () => {
		await mount("idle", "vendor_name", {
			change: chain(
				patchField("vendor_name", { kind: "pairs", required: false }),
				patchRail({ values: { vendor_name: [] } }),
			),
		});
		expect(byRole("button", "Add row").getAttribute("data-fw-focus")).toBe(
			"field:vendor_name",
		);
	});
});

describe("JSON and unsupported types", () => {
	test("a JSON box is a mono text box; ↵ adds a line and the hint says JSON", async () => {
		const store = await mount("idle", "vendor_name", {
			change: patchField("vendor_name", { kind: "json", required: false }),
		});
		const box = byRole("textbox", "Vendor");
		expect(box.className).toContain("font-mono");
		byText("JSON");
		const enter = await press(box, "Enter");
		expect(enter.defaultPrevented).toBe(false);
		await typeInto(box, '{"a":1}');
		expect(store.fake.argsOf("setValue")).toEqual([["vendor_name", '{"a":1}']]);
		expect(store.fake.argsOf("enter")).toEqual([]);
	});

	test("invalid JSON is a sentence", async () => {
		await mount("idle", "vendor_name", {
			change: chain(
				patchField("vendor_name", { kind: "json" }),
				patchRail({ problems: { vendor_name: { code: "json" } } }),
			),
		});
		expect(byRole("alert").textContent).toBe("Enter valid JSON.");
	});

	test("an unknown type is a note that can still take the focus", async () => {
		await mount("idle", "vendor_name", {
			change: chain(
				patchField("vendor_name", {
					kind: "unsupported",
				} as Partial<WorkbenchField>),
				patchRail({ problems: { vendor_name: { code: "unsupported" } } }),
			),
		});
		const note = byText("This kind of input can't be filled in here.");
		expect(note.getAttribute("data-fw-focus")).toBe("field:vendor_name");
		expect(byRole("alert").textContent).toBe(
			"Vendor can't be filled in on this page.",
		);
	});
});
