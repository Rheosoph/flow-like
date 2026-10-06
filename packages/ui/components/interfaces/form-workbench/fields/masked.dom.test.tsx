import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { FormSessionState, WorkbenchField } from "../contracts";
import {
	byRole,
	click,
	installWorkbenchDom,
	mountWorkbench,
	queryByRole,
	typeInto,
} from "../testing/dom";
import { press } from "./field-events";

const dom = installWorkbenchDom();
const { LiveField, liveStore, patchField } = await import("./field-testing");

afterEach(dom.cleanup);
afterAll(dom.restore);

type Store = ReturnType<typeof liveStore>;

async function mount(
	fixtureName: Parameters<typeof liveStore>[0],
	name: string,
	patch: Partial<WorkbenchField>,
	more: (state: FormSessionState) => FormSessionState = (state) => state,
) {
	const store: Store = liveStore(fixtureName, {
		change: (state) => more(patchField(name, patch)(state)),
	});
	await mountWorkbench(<LiveField store={store} name={name} />, {
		layout: store.layout,
	});
	return store;
}

const field = () => document.querySelector("input") as HTMLInputElement;

describe("a sensitive text (spec M6)", () => {
	test("a masked input with Show and Hide, a Tab stop of its own", async () => {
		await mount("reopen", "vendor_name", { sensitive: true });
		expect(field().type).toBe("password");
		expect(field().getAttribute("autocomplete")).toBe("off");
		expect(field().getAttribute("data-fw-focus")).toBe("field:vendor_name");
		const toggle = byRole("button", "Show Vendor");
		expect(toggle.textContent).toBe("Show");
		expect(toggle.getAttribute("aria-pressed")).toBe("false");
		expect(toggle.getAttribute("tabindex")).toBeNull();
		await click(toggle);
		expect(field().type).toBe("text");
		const hide = byRole("button", "Hide Vendor");
		expect(hide.textContent).toBe("Hide");
		expect(hide.getAttribute("aria-pressed")).toBe("true");
		await click(hide);
		expect(field().type).toBe("password");
	});

	test("never a suggestion, a list or a combobox, even with history", async () => {
		const store = await mount("reopen", "vendor_name", { sensitive: true });
		expect(queryByRole("combobox")).toBeNull();
		expect(field().getAttribute("placeholder")).toBeNull();
		await typeInto(field(), "N");
		expect(field().value).toBe("N");
		const down = await press(field(), "ArrowDown");
		expect(down.defaultPrevented).toBe(false);
		expect(store.fake.argsOf("openList")).toEqual([]);
		expect(store.fake.argsOf("setValue")).toEqual([["vendor_name", "N"]]);
	});

	test("↵ moves on and blur validates", async () => {
		const store = await mount("idle", "vendor_name", { sensitive: true });
		const event = await press(field(), "Enter");
		expect(event.defaultPrevented).toBe(true);
		expect(store.fake.argsOf("enter")).toEqual([["vendor_name"]]);
		await press(field(), "Enter", { isComposing: true });
		expect(store.fake.argsOf("enter")).toHaveLength(1);
	});

	test("an optional field with a withheld default says the app's own value is used", async () => {
		await mount("idle", "vendor_name", {
			sensitive: true,
			defaultOmitted: true,
			required: false,
		});
		expect(field().getAttribute("placeholder")).toBe(
			"The app's own value is used when this is empty.",
		);
	});

	test("a required field with a withheld default stays required and says why", async () => {
		await mount(
			"idle",
			"vendor_name",
			{ sensitive: true, defaultOmitted: true },
			(state) => ({
				...state,
				rail: {
					...state.rail,
					problems: { vendor_name: { code: "required" } },
				},
			}),
		);
		expect(byRole("alert").textContent).toBe(
			"Enter Vendor. The app's own value is not shown here.",
		);
		expect(field().getAttribute("placeholder")).toBeNull();
	});

	test("a sensitive number is masked too, in mono and right-aligned", async () => {
		const store = await mount("idle", "expected_total", { sensitive: true });
		expect(field().type).toBe("password");
		expect(field().getAttribute("inputmode")).toBe("decimal");
		expect(field().className).toContain("text-right");
		await typeInto(field(), "12");
		expect(store.fake.argsOf("setValue")).toEqual([["expected_total", "12"]]);
		byRole("button", "Show Expected total");
	});
});
