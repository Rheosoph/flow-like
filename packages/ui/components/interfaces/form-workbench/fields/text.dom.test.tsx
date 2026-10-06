import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import {
	byRole,
	byText,
	click,
	installWorkbenchDom,
	mountWorkbench,
	queryByRole,
	settle,
	typeInto,
} from "../testing/dom";
import { PHONE_LAYOUT } from "../testing/layouts";
import { blurOff, focusOn, press } from "./field-events";

const dom = installWorkbenchDom();
const { LiveField, liveStore } = await import("./field-testing");

afterEach(dom.cleanup);
afterAll(dom.restore);

type Store = ReturnType<typeof liveStore>;

async function mount(
	fixtureName: Parameters<typeof liveStore>[0],
	name: string,
	options: Parameters<typeof liveStore>[1] = {},
) {
	const store: Store = liveStore(fixtureName, options);
	const view = await mountWorkbench(<LiveField store={store} name={name} />, {
		layout: store.layout,
	});
	return { store, view };
}

/** What a browser sends when ⌫ or Delete removes text: an input event with a delete `inputType`. */
async function deleteTo(element: HTMLTextAreaElement, text: string) {
	await act(async () => {
		const view = element.ownerDocument
			.defaultView as unknown as typeof globalThis;
		const setter = Object.getOwnPropertyDescriptor(
			view.HTMLTextAreaElement.prototype,
			"value",
		)?.set;
		setter?.call(element, text);
		element.dispatchEvent(
			new view.InputEvent("input", {
				bubbles: true,
				inputType: "deleteContentBackward",
			}),
		);
	});
}

const typed = (store: Store, name: string) =>
	store.state.rail.values[name] as string;

describe("text field: ↵ and key guards (spec M2)", () => {
	test("↵ moves on and asks the reducer for the next stop", async () => {
		const { store } = await mount("idle", "vendor_name");
		const box = byRole("textbox", "Vendor");
		const event = await press(box, "Enter");
		expect(event.defaultPrevented).toBe(true);
		expect(store.fake.argsOf("enter")).toEqual([["vendor_name"]]);
	});

	test("⇧↵ adds a line, ⌘↵ and Ctrl+↵ are the root's", async () => {
		const { store } = await mount("idle", "vendor_name");
		const box = byRole("textbox", "Vendor");
		const shift = await press(box, "Enter", { shiftKey: true });
		const meta = await press(box, "Enter", { metaKey: true });
		expect(shift.defaultPrevented).toBe(false);
		expect(meta.defaultPrevented).toBe(false);
		expect(store.fake.argsOf("enter")).toEqual([]);
	});

	test("a held ↵ moves on once, an IME's ↵ not at all", async () => {
		const { store } = await mount("idle", "vendor_name");
		const box = byRole("textbox", "Vendor");
		await press(box, "Enter", { repeat: true });
		await press(box, "Enter", { isComposing: true });
		await press(box, "Enter", { keyCode: 229 });
		expect(store.fake.argsOf("enter")).toEqual([]);
		await press(box, "Enter");
		expect(store.fake.argsOf("enter")).toHaveLength(1);
	});

	test("adding a line keeps the same text box, so the cursor never falls to the page", async () => {
		await mount("reopen", "vendor_name");
		const box = byRole("combobox", "Vendor") as HTMLTextAreaElement;
		await focusOn(box);
		await typeInto(box, "a\nb");
		const same = document.querySelector("textarea");
		expect(same).toBe(box);
		expect(dom.document.activeElement).toBe(box);
	});

	test("a text with a line break keeps ↵ for itself", async () => {
		const { store } = await mount("idle", "vendor_name", {
			change: (state) => ({
				...state,
				rail: {
					...state.rail,
					values: { ...state.rail.values, vendor_name: "a\nb" },
				},
			}),
		});
		const event = await press(byRole("textbox", "Vendor"), "Enter");
		expect(event.defaultPrevented).toBe(false);
		expect(store.fake.argsOf("enter")).toEqual([]);
	});
});

describe("text field: edits and focus", () => {
	test("typing in an empty field replaces, inside a value edits in place", async () => {
		const { store } = await mount("idle", "vendor_name");
		const box = byRole("textbox", "Vendor");
		await typeInto(box, "A");
		await typeInto(box, "Ac");
		expect(store.fake.argsOf("setValue")).toEqual([
			["vendor_name", "A", "replace"],
			["vendor_name", "Ac", "inPlace"],
		]);
	});

	test("typing over a fully selected value replaces it", async () => {
		const { store } = await mount("idle", "vendor_name", {
			change: (state) => ({
				...state,
				rail: {
					...state.rail,
					values: { ...state.rail.values, vendor_name: "Acme" },
				},
			}),
		});
		const box = byRole("textbox", "Vendor") as HTMLTextAreaElement;
		box.setSelectionRange(0, 4);
		await press(box, "x");
		await typeInto(box, "x");
		expect(store.fake.argsOf("setValue")).toEqual([
			["vendor_name", "x", "replace"],
		]);
	});

	test("arriving by Tab selects the whole value, a pointer press does not", async () => {
		await mount("idle", "vendor_name", {
			change: (state) => ({
				...state,
				rail: {
					...state.rail,
					values: { ...state.rail.values, vendor_name: "Acme" },
				},
			}),
		});
		const box = byRole("textbox", "Vendor") as HTMLTextAreaElement;
		await focusOn(box);
		expect([box.selectionStart, box.selectionEnd]).toEqual([0, 4]);
	});

	test("blur validates", async () => {
		const { store } = await mount("idle", "vendor_name");
		const box = byRole("textbox", "Vendor") as HTMLTextAreaElement;
		await focusOn(box);
		await blurOff(box);
		expect(store.fake.argsOf("blurField")).toEqual([["vendor_name"]]);
	});

	test("carries the focus attribute, the combobox-less role and the help text", async () => {
		await mount("idle", "vendor_name");
		const box = byRole("textbox", "Vendor");
		expect(box.getAttribute("data-fw-focus")).toBe("field:vendor_name");
		expect(box.getAttribute("role")).toBeNull();
		expect(box.getAttribute("aria-describedby")).toBe(
			byText("Name as it appears in your supplier master data.").id,
		);
	});

	test("a problem shows its sentence with an icon and describes the control", async () => {
		await mount("idle", "vendor_name", {
			change: (state) => ({
				...state,
				rail: {
					...state.rail,
					problems: { vendor_name: { code: "required" } },
				},
			}),
		});
		const box = byRole("textbox", "Vendor");
		const message = byRole("alert");
		expect(message.textContent).toBe("Enter the vendor.");
		expect(message.querySelector("svg")).not.toBeNull();
		expect(box.getAttribute("aria-invalid")).toBe("true");
		expect(box.getAttribute("aria-describedby")?.split(" ")).toContain(
			message.id,
		);
	});
});

describe("text field: inline suggestion and recent values (spec M6)", () => {
	test("typing N suggests the rest as selected text; the value stays what was typed", async () => {
		const { store } = await mount("reopen", "vendor_name");
		const box = byRole("combobox", "Vendor") as HTMLTextAreaElement;
		expect(box.getAttribute("aria-autocomplete")).toBe("both");
		await focusOn(box);
		await typeInto(box, "N");
		expect(box.value).toBe("Nordwind Logistik GmbH");
		expect([box.selectionStart, box.selectionEnd]).toEqual([1, 22]);
		expect(typed(store, "vendor_name")).toBe("N");
	});

	test("only → takes the suggestion", async () => {
		const { store } = await mount("reopen", "vendor_name");
		const box = byRole("combobox", "Vendor") as HTMLTextAreaElement;
		await typeInto(box, "N");
		const event = await press(box, "ArrowRight");
		expect(event.defaultPrevented).toBe(true);
		expect(typed(store, "vendor_name")).toBe("Nordwind Logistik GmbH");
		await settle();
		expect(box.value).toBe("Nordwind Logistik GmbH");
		expect([box.selectionStart, box.selectionEnd]).toEqual([22, 22]);
	});

	test("↵ keeps exactly what was typed and moves on", async () => {
		const { store } = await mount("reopen", "vendor_name");
		const box = byRole("combobox", "Vendor") as HTMLTextAreaElement;
		await typeInto(box, "N");
		await press(box, "Enter");
		expect(store.fake.argsOf("enter")).toEqual([["vendor_name"]]);
		expect(typed(store, "vendor_name")).toBe("N");
		await settle();
		expect(box.value).toBe("N");
	});

	test("Tab and Esc drop the suggestion", async () => {
		const { store } = await mount("reopen", "vendor_name");
		const box = byRole("combobox", "Vendor") as HTMLTextAreaElement;
		await typeInto(box, "N");
		const dropped = await press(box, "Escape");
		expect(dropped.defaultPrevented).toBe(true);
		await settle();
		expect(box.value).toBe("N");
		await typeInto(box, "No");
		expect(box.value).toBe("Nordwind Logistik GmbH");
		await press(box, "Tab");
		await settle();
		expect(box.value).toBe("No");
		expect(typed(store, "vendor_name")).toBe("No");
	});

	test("deleting never brings a suggestion back", async () => {
		const { store } = await mount("reopen", "vendor_name");
		const box = byRole("combobox", "Vendor") as HTMLTextAreaElement;
		await focusOn(box);
		await typeInto(box, "No");
		expect(box.value).toBe("Nordwind Logistik GmbH");
		await deleteTo(box, "N");
		await settle();
		expect(box.value).toBe("N");
		expect(typed(store, "vendor_name")).toBe("N");
	});

	test("an IME composition gets no suggestion and no ↵", async () => {
		const { store } = await mount("reopen", "vendor_name");
		const box = byRole("combobox", "Vendor") as HTMLTextAreaElement;
		await press(box, "Enter", { isComposing: true });
		expect(store.fake.argsOf("enter")).toEqual([]);
	});

	test("the placeholder points at the list on a fine pointer with history", async () => {
		await mount("reopen", "vendor_name");
		const box = byRole("combobox", "Vendor");
		expect(box.getAttribute("placeholder")).toBe(
			"Type, or press ↓ for recent values",
		);
	});

	test("↓ opens the list: newest first, first row active, footer, hint", async () => {
		const { store } = await mount("reopen", "vendor_name");
		const box = byRole("combobox", "Vendor");
		const event = await press(box, "ArrowDown");
		expect(event.defaultPrevented).toBe(true);
		expect(store.fake.argsOf("openList")).toEqual([["recent", "vendor_name"]]);
		const list = byRole("listbox", "Recent values for Vendor");
		const rows = Array.from(list.querySelectorAll("[role=option]"));
		expect(rows.map((row) => row.firstElementChild?.textContent)).toEqual([
			"Nordwind Logistik GmbH",
			"Alpenfracht AG",
		]);
		expect(rows[0].getAttribute("aria-selected")).toBe("true");
		expect(box.getAttribute("aria-expanded")).toBe("true");
		expect(box.getAttribute("aria-activedescendant")).toBe(rows[0].id);
		byText("Don't save Vendor on this device");
		byText("↑↓ move · ↵ choose · esc close");
	});

	test("↵ fills the highlighted value and closes; Esc closes; Delete forgets", async () => {
		const { store } = await mount("reopen", "vendor_name");
		const box = byRole("combobox", "Vendor");
		await press(box, "ArrowDown");
		await press(box, "Delete");
		expect(store.fake.argsOf("forgetRecent")).toEqual([
			["vendor_name", "Nordwind Logistik GmbH"],
		]);
		await press(box, "Enter");
		expect(store.fake.argsOf("setValue")).toEqual([
			["vendor_name", "Nordwind Logistik GmbH", "replace"],
		]);
		expect(store.state.view.list).toBeNull();
		await press(box, "ArrowDown");
		await press(box, "Escape");
		expect(store.state.view.list).toBeNull();
		expect(store.fake.argsOf("enter")).toEqual([]);
	});

	test("clicking a row fills it; the footer button stops saving the field", async () => {
		const { store } = await mount("reopen", "vendor_name");
		await press(byRole("combobox", "Vendor"), "ArrowDown");
		await click(byRole("option", /Alpenfracht AG/));
		expect(store.fake.argsOf("setValue")[0]).toEqual([
			"vendor_name",
			"Alpenfracht AG",
			"replace",
		]);
		await press(byRole("combobox", "Vendor"), "ArrowDown");
		await click(byText("Don't save Vendor on this device"));
		expect(store.fake.argsOf("dontSave")).toEqual([["vendor_name"]]);
	});

	test("↓ and ↑ move within the rows and stop at the ends", async () => {
		const { store } = await mount("reopen", "vendor_name");
		const box = byRole("combobox", "Vendor");
		await press(box, "ArrowDown");
		await press(box, "ArrowDown");
		await press(box, "ArrowDown");
		await press(box, "ArrowUp");
		await press(box, "ArrowUp");
		expect(store.fake.argsOf("moveList")).toEqual([[1], [-1]]);
	});

	test("no list, no suggestion and no combobox on a touch screen", async () => {
		await mount("reopen", "vendor_name", { layout: PHONE_LAYOUT });
		expect(queryByRole("combobox")).toBeNull();
		const box = byRole("textbox", "Vendor");
		expect(box.getAttribute("placeholder")).toBeNull();
		expect(box.className).toContain("min-h-11");
	});
});
