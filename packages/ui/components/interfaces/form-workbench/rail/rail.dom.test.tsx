import {
	afterAll,
	afterEach,
	describe,
	expect,
	mock,
	setSystemTime,
	test,
} from "bun:test";
import { type ReactNode, act } from "react";
import type {
	FieldControlProps,
	FormSessionState,
	InterfaceModalProps,
	Preset,
	RailProps,
	WorkbenchLayout,
} from "../contracts";
import {
	advance,
	allByRole,
	byRole,
	byText,
	click,
	inPortal,
	installWorkbenchDom,
	keyDown,
	mountWorkbench,
	queryByRole,
	settle,
	typeInto,
} from "../testing/dom";
import { fakeView } from "../testing/fake-actions";
import { fixture, fixtureClock } from "../testing/fixtures";
import { DESKTOP_LAYOUT, PHONE_LAYOUT } from "../testing/layouts";

/*
 * The rail against fixtures (PLAN §12.10). Field controls and the modal frame belong to sibling
 * lanes, so both are replaced by small stand-ins: the controls keep their props for assertions and
 * carry the focus attribute, the frame renders its children inline. DOM files run one per process.
 */

const seen: FieldControlProps[] = [];

mock.module("../fields/field-control", () => ({
	FieldControl: (props: FieldControlProps) => {
		seen.push(props);
		return (
			<button type="button" data-fw-focus={`field:${props.field.key}`}>
				{props.field.label}
			</button>
		);
	},
}));

mock.module("../shell/interface-modal", () => ({
	InterfaceModal: (props: InterfaceModalProps): ReactNode =>
		props.open ? (
			<dialog open aria-labelledby={props.labelledBy}>
				{props.children}
			</dialog>
		) : null,
}));

const dom = installWorkbenchDom();
const { Rail } = await import("./rail");
const { clearScrollMemory } = await import("./use-scroll-memory");

afterEach(async () => {
	seen.length = 0;
	clearScrollMemory();
	await dom.cleanup();
});
afterAll(() => dom.restore());

async function show(
	state: FormSessionState,
	layout: WorkbenchLayout = DESKTOP_LAYOUT,
) {
	const view = fakeView(state, { layout });
	const mounted = await mountWorkbench(
		<Rail {...(view.props as RailProps)} />,
		{
			layout,
		},
	);
	return { ...mounted, view };
}

const cells = (container: HTMLElement) =>
	Array.from(container.querySelectorAll<HTMLElement>("[data-rail-field]"));
const cellNames = (container: HTMLElement) =>
	cells(container).map((cell) => cell.dataset.railField);
const lastProps = (key: string): FieldControlProps => {
	const found = [...seen].reverse().find((props) => props.field.key === key);
	if (!found) throw new Error(`no control was drawn for ${key}`);
	return found;
};

const withRail = (
	state: FormSessionState,
	patch: Partial<FormSessionState["rail"]>,
): FormSessionState => ({ ...state, rail: { ...state.rail, ...patch } });

const withView = (
	state: FormSessionState,
	patch: Partial<FormSessionState["view"]>,
): FormSessionState => ({ ...state, view: { ...state.view, ...patch } });

describe("the head and the field list", () => {
	test("a first visit: the name, the nine fields, nothing else", async () => {
		const { container } = await show(fixture("idle"));
		expect(byRole("heading", "Extract invoice")).toBeTruthy();
		expect(queryByRole("button", /^Runs/)).toBeNull();
		expect(queryByRole("button", /Reset to defaults/)).toBeNull();
		expect(queryByRole("textbox", "Filter fields")).toBeNull();
		expect(cells(container)).toHaveLength(9);
	});

	test("short fields pair, long ones take the row", async () => {
		const { container } = await show(fixture("idle"));
		const cell = (name: string) =>
			container.querySelector<HTMLElement>(`[data-rail-field="${name}"]`);
		expect(cell("vendor_name")?.dataset.fwCell).toBe("full");
		expect(cell("invoice_date")?.dataset.fwCell).toBe("half");
		expect(cell("expected_total")?.dataset.fwCell).toBe("half");
		expect(cell("max_pages")?.dataset.fwCell).toBe("half");
	});

	test("every control gets the markers, the host, the viewer and today", async () => {
		await show(fixture("done"));
		const total = lastProps("expected_total");
		expect(total.markersFor("expected_total").changed).toBe(true);
		expect(total.markersFor("vendor_name").changed).toBe(false);
		expect(total.host.kind).toBe("app");
		expect(total.viewer.mac).toBe(true);
		expect(total.today).toMatch(/^\d{4}-\d{2}-\d{2}$/);
		expect(total.blocked).toBe(false);
		expect(total.disabled).toBe(false);
		expect(total.list).toBeNull();
	});

	test("a FlowPath field this host cannot fill is blocked", async () => {
		await show(fixture("hosted-files"));
		expect(lastProps("invoice_file").blocked).toBe(true);
		expect(lastProps("vendor_name").blocked).toBe(false);
	});

	test("the open list reaches only its own control", async () => {
		const list = { kind: "recent", key: "vendor_name", active: 0 } as const;
		await show(withView(fixture("idle"), { list }));
		expect(lastProps("vendor_name").list).toEqual(list);
		expect(lastProps("max_pages").list).toBeNull();
	});

	test("the control reads recent values, dates and Enter hints through the model", async () => {
		await show(fixture("done"));
		const control = lastProps("vendor_name");
		expect(control.recallFor("vendor_name")?.source.length).toBeGreaterThan(0);
		expect(control.recallFor("expected_total")).toBeNull();
		expect(control.dateAnchorFor("invoice_date")).toMatch(
			/^\d{4}-\d{2}-\d{2}$/,
		);
		expect(["next", "go"]).toContain(control.enterHint("vendor_name"));
	});

	test("a touch screen gets no recent values", async () => {
		await show(fixture("done"), PHONE_LAYOUT);
		expect(lastProps("vendor_name").recallFor("vendor_name")).toBeNull();
	});

	test("the field list is a form that never submits", async () => {
		const { container } = await show(fixture("idle"));
		const form = container.querySelector("form");
		expect(form?.getAttribute("novalidate")).not.toBeNull();
		const event = new dom.window.Event("submit", {
			bubbles: true,
			cancelable: true,
		});
		form?.dispatchEvent(event as unknown as Event);
		expect(event.defaultPrevented).toBe(true);
	});

	test("Reset to defaults shows while something differs and acts through the session", async () => {
		const { view } = await show(fixture("done"));
		await click(byRole("button", "Reset to defaults"));
		expect(view.calls.map((call) => call.name)).toEqual(["resetAll"]);
	});
});

describe("Inputs | Runs", () => {
	test("from two runs, with the count; a tap switches", async () => {
		const { view } = await show(fixture("done"));
		const runs = byRole("button", /^Runs\s*14$/);
		expect(runs.getAttribute("aria-pressed")).toBe("false");
		await click(runs);
		expect(view.argsOf("setRailTab")).toEqual([["runs"]]);
	});

	test("the description is clamped to two lines once there is history", async () => {
		await show(fixture("done"));
		const description = byText(
			/^Upload an invoice and its supporting documents/,
		);
		expect(description.className).toContain("line-clamp-2");
		expect(description.getAttribute("title")).toContain("checks the totals");
	});

	test("on a narrow box there are no tabs, whatever the state says", async () => {
		await show(fixture("runs"), PHONE_LAYOUT);
		expect(queryByRole("button", /^Runs/)).toBeNull();
		expect(cells(document.body)).toHaveLength(9);
	});
});

describe("the Runs list", () => {
	test("14 runs under Today, Yesterday and a dated day, the run on the stage marked", async () => {
		setSystemTime(new Date(fixtureClock("runs").now));
		try {
			const { container } = await show(fixture("runs"));
			expect(container.querySelectorAll("[data-run-row]")).toHaveLength(14);
			const headings = Array.from(container.querySelectorAll("h3")).map(
				(node) => node.textContent?.trim(),
			);
			expect(headings).toEqual(["Today", "Yesterday", "Mon 28 Sep"]);
			expect(byText("Runs on this device")).toBeTruthy();
			const current = container.querySelector('[aria-current="true"]');
			expect(current?.getAttribute("data-run-id")).toBe("run-14");
			expect(cells(container)).toHaveLength(0);
		} finally {
			setSystemTime();
		}
	});

	test("a row says its time, how long it took and what changed", async () => {
		const { container } = await show(fixture("runs"));
		const row = container.querySelector<HTMLElement>('[data-run-id="run-14"]');
		expect(row?.textContent).toContain("Run 14");
		expect(row?.textContent).toMatch(/14:02 · 48 s/);
		expect(row?.textContent).toContain("Run OCR: Off → On");
		const failed = container.querySelector<HTMLElement>(
			'[data-run-id="run-13"]',
		);
		expect(failed?.textContent).toContain("Failed");
		expect(failed?.textContent).toContain("Failed at “Run OCR”.");
		const oldest = container.querySelector<HTMLElement>(
			'[data-run-id="run-1"]',
		);
		expect(oldest?.textContent).toContain("First run");
	});

	test("a finished run has an accessible Done, not a visible word", async () => {
		const { container } = await show(fixture("runs"));
		const icon = container.querySelector<HTMLElement>(
			'[data-run-id="run-14"] [role="img"]',
		);
		expect(icon?.getAttribute("aria-label")).toBe("Done");
	});

	test("picking a row puts that run on the stage", async () => {
		const { view, container } = await show(fixture("runs"));
		const row = container.querySelector<HTMLElement>('[data-run-id="run-13"]');
		if (!row) throw new Error("no row");
		await click(row);
		expect(view.argsOf("selectRun")).toEqual([["run-13", "list"]]);
	});

	test("↓ moves to the next row, End to the last", async () => {
		const { container } = await show(fixture("runs"));
		const rows = Array.from(
			container.querySelectorAll<HTMLElement>("[data-run-row]"),
		);
		const last = rows.at(-1) as HTMLElement;
		rows[0]?.focus();
		await keyDown(rows[0] as HTMLElement, "ArrowDown");
		expect(document.activeElement).toBe(rows[1] ?? null);
		await keyDown(rows[1] as HTMLElement, "End");
		expect(document.activeElement).toBe(last);
		await keyDown(last, "ArrowDown");
		expect(document.activeElement).toBe(last);
	});

	test("queued runs head Today with their place in line", async () => {
		const { container, unmount } = await show(
			withRail(fixture("series"), { tab: "runs" }),
		);
		const first = container.querySelector<HTMLElement>("[data-run-row]");
		expect(first?.textContent).toContain("Queued");
		expect(first?.textContent).toContain("1st in line");
		expect(container.querySelectorAll("h3")[0]?.textContent).toBe("Today");
		await unmount();
	});

	test("a running run shows a clock that ticks once a second", async () => {
		const state = withRail(fixture("series"), { tab: "runs" });
		const started = state.runs.find((run) => run.id === "run-17")?.startedAt;
		if (started == null) throw new Error("run 17 has not started");
		const textOf = (container: HTMLElement) =>
			container.querySelector('[data-run-id="run-17"]')?.textContent ?? "";
		setSystemTime(new Date(started + 300_000));
		try {
			const { container, unmount } = await show(state);
			expect(textOf(container)).toContain("· 5:00");
			setSystemTime(new Date(started + 302_000));
			await advance(1100);
			expect(textOf(container)).toContain("· 5:02");
			await unmount();
		} finally {
			setSystemTime();
		}
	});

	test("the Runs tab keeps the filter row away", async () => {
		const large = fixture("large");
		const state = withRail(
			{ ...large, runs: fixture("runs").runs },
			{ tab: "runs" },
		);
		await show(state);
		expect(queryByRole("textbox", "Filter fields")).toBeNull();
	});
});

describe("the filter (S5)", () => {
	test("it starts from twelve fields, with its chips and counts", async () => {
		await show(fixture("large"));
		expect(byRole("textbox", "Filter fields")).toBeTruthy();
		const group = byRole("group", "Which fields to show");
		const labels = Array.from(group.querySelectorAll("button")).map((button) =>
			button.textContent?.replace(/\s+/g, " ").trim(),
		);
		expect(labels).toEqual(["All 29", "Required 5", "Changed 0"]);
	});

	test("typing and the chips go to the session", async () => {
		const { view } = await show(fixture("large"));
		await typeInto(byRole("textbox", "Filter fields"), "cust");
		expect(view.argsOf("setFilter")).toEqual([[{ query: "cust" }]]);
		await click(byRole("button", /^Required/));
		expect(view.argsOf("setFilter").at(-1)).toEqual([{ chip: "required" }]);
	});

	test("the text and the chip narrow the list; counts stay", async () => {
		const large = fixture("large");
		const { container } = await show(
			withRail(large, { filter: { query: "customer", chip: "all" } }),
		);
		expect(cellNames(container)).toContain("customer_number");
		expect(cells(container).length).toBeLessThan(29);
		const required = await show(
			withRail(large, { filter: { query: "", chip: "required" } }),
		);
		expect(cells(required.container).length).toBeGreaterThanOrEqual(5);
	});

	test("nothing matches: a line and a way back", async () => {
		const { view, container } = await show(
			withRail(fixture("large"), { filter: { query: "zzzz", chip: "all" } }),
		);
		expect(cells(container)).toHaveLength(0);
		expect(byText("No field matches.")).toBeTruthy();
		await click(byRole("button", "Show all fields"));
		expect(view.argsOf("setFilter")).toEqual([[{ query: "", chip: "all" }]]);
	});

	test("↵ puts the cursor in the first match and keeps the filter", async () => {
		const { view } = await show(
			withRail(fixture("large"), {
				filter: { query: "customer", chip: "all" },
			}),
		);
		const box = byRole("textbox", "Filter fields");
		box.focus();
		await keyDown(box, "Enter");
		const active = document.activeElement as HTMLElement | null;
		expect(active?.getAttribute("data-fw-focus")).toBe("field:customer_number");
		expect(view.argsOf("setFilter")).toEqual([]);
	});

	test("Esc clears the text; with nothing typed it is left alone", async () => {
		const { view } = await show(
			withRail(fixture("large"), { filter: { query: "cust", chip: "all" } }),
		);
		await keyDown(byRole("textbox", "Filter fields"), "Escape");
		expect(view.argsOf("setFilter")).toEqual([[{ query: "" }]]);
		view.clear();
		await dom.cleanup();
		const again = await show(fixture("large"));
		await keyDown(byRole("textbox", "Filter fields"), "Escape");
		expect(again.view.argsOf("setFilter")).toEqual([]);
	});

	test("the box carries the focus attribute and the key it answers to", async () => {
		await show(fixture("large"));
		const box = byRole("textbox", "Filter fields");
		expect(box.getAttribute("data-fw-focus")).toBe("filter");
		expect(box.getAttribute("aria-keyshortcuts")).toBe("/");
		expect(byText("/")).toBeTruthy();
	});

	test("a field the session wants focus on is never filtered away", async () => {
		const state = withView(
			withRail(fixture("large"), { filter: { query: "zzzz", chip: "all" } }),
			{
				focus: {
					seq: 1,
					target: {
						kind: "field",
						key: "customer_number",
						select: true,
						scrollOnly: false,
					},
				},
			},
		);
		const { view, container } = await show(state);
		expect(cellNames(container)).toEqual(["customer_number"]);
		expect(view.argsOf("setFilter")).toEqual([[{ query: "", chip: "all" }]]);
	});

	test("a field that holds the cursor stays while it stops matching Changed", async () => {
		const large = fixture("large");
		const field = large.form.fields.find(
			(item) => item.kind === "number" && item.hasDefault,
		);
		if (!field) throw new Error("the large form has no numeric default");
		const changedRail = {
			...large.rail.values,
			[field.name]: `${field.defaultValue}1`,
		};
		const chip = { filter: { query: "", chip: "changed" as const } };
		const edited = withRail(large, { ...chip, values: changedRail });
		const reset = withRail(large, { ...chip, values: large.rail.values });
		const { container, rerender } = await show(edited);
		expect(cellNames(container)).toEqual([field.name]);
		const control = container.querySelector<HTMLElement>(
			'[data-fw-focus^="field:"]',
		);
		await act(async () => control?.focus());
		const view = fakeView(reset, { layout: DESKTOP_LAYOUT });
		await rerender(<Rail {...(view.props as RailProps)} />);
		expect(cellNames(container)).toEqual([field.name]);
		await act(async () => control?.blur());
		expect(cellNames(container)).toEqual([]);
	});
});

describe("keys in the rail", () => {
	const withStartMessage = () => {
		const state = fixture("series");
		if (!state.view.message) throw new Error("series fixture has no message");
		return state;
	};

	test("a key press ends a start message early", async () => {
		const { view, container } = await show(withStartMessage());
		const field = container.querySelector<HTMLElement>("[data-fw-focus]");
		if (!field) throw new Error("no field");
		await keyDown(field, "a");
		expect(view.argsOf("railKey")).toHaveLength(1);
	});

	test("a bare modifier does not", async () => {
		const { view, container } = await show(withStartMessage());
		const field = container.querySelector<HTMLElement>("[data-fw-focus]");
		if (!field) throw new Error("no field");
		await keyDown(field, "Shift");
		await keyDown(field, "Meta");
		expect(view.argsOf("railKey")).toHaveLength(0);
	});

	test("no message, no call", async () => {
		const { view, container } = await show(fixture("idle"));
		const field = container.querySelector<HTMLElement>("[data-fw-focus]");
		if (!field) throw new Error("no field");
		await keyDown(field, "a");
		expect(view.argsOf("railKey")).toHaveLength(0);
	});
});

describe("the Presets button and menu (S1)", () => {
	test("no preset and a small form: no button at all", async () => {
		await show(fixture("done"));
		expect(queryByRole("button", /Open presets/)).toBeNull();
	});

	test("the active preset names the button", async () => {
		await show(withView(fixture("presets"), { overlay: null }));
		const button = byRole("button", /Open presets/);
		expect(button.getAttribute("aria-label")).toBe(
			"Preset: Nordwind Logistik GmbH. Open presets",
		);
		expect(button.getAttribute("title")).toBe("Presets · ⌘P");
		expect(button.textContent).toContain("Nordwind Logistik GmbH");
	});

	test("an edited preset says how many inputs", async () => {
		const state = withView(fixture("presets"), { overlay: null });
		const edited = withRail(state, {
			values: { ...state.rail.values, vendor_name: "Somebody else" },
		});
		await show(edited);
		expect(byRole("button", /Open presets/).getAttribute("aria-label")).toBe(
			"Preset: Nordwind Logistik GmbH, 1 input edited. Open presets",
		);
	});

	test("the button opens the menu through the session", async () => {
		const { view } = await show(
			withView(fixture("presets"), { overlay: null }),
		);
		await click(byRole("button", /Open presets/));
		expect(view.argsOf("openOverlay")).toEqual([[{ id: "presets" }]]);
	});

	test("the open menu lists the presets, the active one checked, with digits", async () => {
		await show(fixture("presets"));
		const menu = inPortal("dialog");
		const rows = Array.from(
			menu.querySelectorAll<HTMLElement>("[data-preset-id]"),
		);
		expect(rows.map((row) => row.dataset.presetId)).toEqual([
			"preset-alpenfracht",
			"preset-nordwind",
		]);
		expect(rows[0]?.textContent).toContain("Alpenfracht AG");
		expect(rows[0]?.textContent).toContain(
			"Vendor Alpenfracht AG · Max pages 60",
		);
		expect(rows[1]?.getAttribute("aria-current")).toBe("true");
		expect(rows[1]?.textContent).toContain("On open");
		expect(rows[0]?.textContent).toMatch(/^Alpenfracht AG1Vendor/);
		expect(rows[1]?.textContent).toMatch(/On open2Vendor/);
		expect(byText("Save as new preset…")).toBeTruthy();
		expect(byText("⌘S")).toBeTruthy();
		expect(menu.querySelector('input[type="search"]')).toBeNull();
	});

	test("the active preset's row has the focus; a digit applies that preset", async () => {
		const { view } = await show(fixture("presets"));
		const menu = inPortal("dialog");
		await settle();
		expect(document.activeElement?.getAttribute("data-preset-id")).toBe(
			"preset-nordwind",
		);
		await keyDown(document.activeElement as HTMLElement, "1");
		expect(view.argsOf("applyPreset")).toEqual([["preset-alpenfracht"]]);
		expect(view.argsOf("closeOverlay")).toHaveLength(1);
		expect(menu).toBeTruthy();
	});

	test("a digit nobody holds does nothing; modifiers are left to the shell", async () => {
		const { view } = await show(fixture("presets"));
		await settle();
		const focused = document.activeElement as HTMLElement;
		await keyDown(focused, "7");
		await keyDown(focused, "1", { metaKey: true });
		expect(view.argsOf("applyPreset")).toEqual([]);
	});

	test("a click applies; the cross deletes", async () => {
		const { view } = await show(fixture("presets"));
		const menu = inPortal("dialog");
		await click(
			menu.querySelector<HTMLElement>(
				'[data-preset-id="preset-alpenfracht"]',
			) as HTMLElement,
		);
		expect(view.argsOf("applyPreset")).toEqual([["preset-alpenfracht"]]);
		await click(byRole("button", "Delete Alpenfracht AG"));
		expect(view.argsOf("deletePreset")).toEqual([["preset-alpenfracht"]]);
	});

	test("Delete on a row deletes that preset", async () => {
		const { view } = await show(fixture("presets"));
		await settle();
		await keyDown(document.activeElement as HTMLElement, "Delete");
		expect(view.argsOf("deletePreset")).toEqual([["preset-nordwind"]]);
	});

	test("↑/↓ move through the rows and the actions", async () => {
		await show(fixture("presets"));
		await settle();
		const menu = inPortal("dialog");
		const items = Array.from(
			menu.querySelectorAll<HTMLElement>("[data-preset-item]"),
		);
		expect(items).toHaveLength(3);
		(items[0] as HTMLElement).focus();
		await keyDown(items[0] as HTMLElement, "ArrowDown");
		expect(document.activeElement).toBe(items[1]);
		await keyDown(items[1] as HTMLElement, "End");
		expect(document.activeElement).toBe(items[2]);
		await keyDown(items[2] as HTMLElement, "Home");
		expect(document.activeElement).toBe(items[0]);
	});

	test("an edited preset offers Update and Reset to", async () => {
		const state = fixture("presets");
		const edited = withRail(state, {
			values: { ...state.rail.values, vendor_name: "Somebody else" },
		});
		const { view } = await show(edited);
		const menu = inPortal("dialog");
		expect(menu.textContent).toContain("Update Nordwind Logistik GmbH");
		expect(menu.textContent).toContain("1 change");
		await click(byText("Update Nordwind Logistik GmbH"));
		expect(view.argsOf("updatePreset")).toEqual([["preset-nordwind"]]);
		await click(byText("Reset to Nordwind Logistik GmbH"));
		expect(view.argsOf("resetToPreset")).toHaveLength(1);
	});

	test("an unedited preset has neither", async () => {
		await show(fixture("presets"));
		const menu = inPortal("dialog");
		expect(menu.textContent).not.toContain("Update ");
		expect(menu.textContent).not.toContain("Reset to ");
	});

	test("Save as new preset opens the Save dialog and leaves the menu to it", async () => {
		const { view } = await show(fixture("presets"));
		await click(byText("Save as new preset…"));
		expect(view.argsOf("openOverlay")).toEqual([
			[{ id: "presetSave", mode: "save", fromRunId: null }],
		]);
		expect(view.argsOf("closeOverlay")).toHaveLength(0);
	});

	test("with no presets: the invitation and the one action", async () => {
		const state = withView(fixture("done"), { overlay: { id: "presets" } });
		const { view } = await show(state);
		const menu = inPortal("dialog");
		expect(menu.textContent).toContain(
			"Save inputs you use often, then fill the form in one step.",
		);
		await click(byText("Save these inputs as a preset…"));
		expect(view.argsOf("openOverlay")).toEqual([
			[{ id: "presetSave", mode: "save", fromRunId: null }],
		]);
	});

	test("from six presets a find box narrows the list", async () => {
		const base = fixture("presets");
		const many: Preset[] = Array.from({ length: 6 }, (_, index) => ({
			...(base.memory.presets[0] as Preset),
			id: `p${index}`,
			name: index === 4 ? "Bergmann KG" : `Customer ${index}`,
			digit: index + 1,
			sets: { vendor_name: `Vendor ${index}` },
			openDefault: false,
		}));
		const state: FormSessionState = {
			...base,
			memory: { ...base.memory, presets: many },
			rail: { ...base.rail, activePresetId: null },
		};
		await show(state);
		const menu = inPortal("dialog");
		const find = menu.querySelector<HTMLInputElement>('input[type="search"]');
		expect(find).not.toBeNull();
		await typeInto(find as HTMLInputElement, "berg");
		const rows = Array.from(
			menu.querySelectorAll<HTMLElement>("[data-preset-id]"),
		);
		expect(rows.map((row) => row.dataset.presetId)).toEqual(["p4"]);
		await typeInto(find as HTMLInputElement, "nothing like it");
		expect(byText("No preset matches.")).toBeTruthy();
	});

	test("on a touch screen the menu is a sheet: no digits, a cross that is always there", async () => {
		const state = withView(fixture("presets"), { overlay: { id: "presets" } });
		await show(state, { ...PHONE_LAYOUT });
		const sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Presets");
		const rows = Array.from(
			sheet.querySelectorAll<HTMLElement>("[data-preset-id]"),
		);
		expect(rows).toHaveLength(2);
		expect(rows[0]?.textContent).toMatch(/^Alpenfracht AGVendor/);
		expect(queryByRole("button", "Delete Alpenfracht AG")).not.toBeNull();
	});

	test("the sheet closes with Esc and with its cross, and takes the cursor itself", async () => {
		const state = withView(fixture("presets"), { overlay: { id: "presets" } });
		const { view } = await show(state, { ...PHONE_LAYOUT });
		const sheet = inPortal("dialog");
		await settle();
		expect(sheet.contains(document.activeElement)).toBe(true);
		expect(document.activeElement?.getAttribute("aria-label")).not.toBe(
			"Close",
		);
		await keyDown(document.activeElement as HTMLElement, "Escape");
		expect(view.argsOf("closeOverlay")).toHaveLength(1);
		await click(byRole("button", "Close"));
		expect(view.argsOf("closeOverlay")).toHaveLength(2);
	});

	test("a menu without a button still opens (⌘P on a small form)", async () => {
		await show(withView(fixture("done"), { overlay: { id: "presets" } }));
		expect(inPortal("dialog")).toBeTruthy();
		expect(queryByRole("button", /Open presets/)).toBeNull();
	});
});

describe("the Save dialog (S1)", () => {
	const open = (state = fixture("preset-save")) => state;

	test("it opens on the proposed name, ticks Vendor and applies on open", async () => {
		await show(open());
		const dialog = inPortal("dialog");
		expect(dialog.textContent).toContain("Save inputs as a preset");
		const name = byRole("textbox", "Name") as HTMLInputElement;
		expect(name.value).toBe("Nordwind Logistik GmbH");
		expect(
			byRole("checkbox", "Apply when this form opens").getAttribute(
				"data-state",
			),
		).toBe("checked");
		expect(dialog.textContent).toContain("1 of 9");
		expect(byRole("checkbox", /^Vendor/).getAttribute("data-state")).toBe(
			"checked",
		);
		const invoice = byRole("checkbox", /^Invoice/);
		expect(invoice.hasAttribute("disabled")).toBe(true);
		expect(dialog.textContent).toContain("Files are not saved");
		expect(dialog.textContent).toContain("At their defaults (7)");
	});

	test("the defaults group opens and its inputs can be ticked", async () => {
		await show(open());
		expect(queryByRole("checkbox", /^Max pages/)).toBeNull();
		await click(byRole("button", /At their defaults/));
		const pages = byRole("checkbox", /^Max pages/);
		await click(pages);
		expect(pages.getAttribute("data-state")).toBe("checked");
		expect(inPortal("dialog").textContent).toContain("2 of 9");
	});

	test("Save sends the draft and closes", async () => {
		const { view } = await show(open());
		await click(byRole("button", "Save preset"));
		expect(view.argsOf("savePreset")).toEqual([
			[
				{
					name: "Nordwind Logistik GmbH",
					openDefault: true,
					ticked: ["vendor_name"],
					fromRunId: null,
					replaceId: null,
				},
			],
		]);
		expect(view.argsOf("closeOverlay")).toHaveLength(1);
	});

	test("an empty name explains itself and saves nothing", async () => {
		const { view } = await show(open());
		await typeInto(byRole("textbox", "Name"), "   ");
		await click(byRole("button", "Save preset"));
		expect(inPortal("dialog").textContent).toContain("Enter a name.");
		expect(view.argsOf("savePreset")).toEqual([]);
	});

	test("nothing ticked explains itself too", async () => {
		const { view } = await show(open());
		await click(byRole("checkbox", /^Vendor/));
		await click(byRole("button", "Save preset"));
		expect(inPortal("dialog").textContent).toContain(
			"Tick at least one input.",
		);
		expect(view.argsOf("savePreset")).toEqual([]);
	});

	test("a name that exists turns the primary into Replace and says why", async () => {
		const base = fixture("preset-save");
		const state: FormSessionState = {
			...base,
			memory: {
				...base.memory,
				presets: fixture("presets").memory.presets,
			},
		};
		const { view } = await show(state);
		await typeInto(byRole("textbox", "Name"), "alpenfracht ag");
		const dialog = inPortal("dialog");
		expect(dialog.textContent).toContain(
			"A preset called Alpenfracht AG exists. Saving replaces it.",
		);
		await click(byRole("button", "Replace Alpenfracht AG"));
		const [draft] = view.argsOf("savePreset")[0] ?? [];
		expect(draft?.replaceId).toBe("preset-alpenfracht");
	});

	test("update mode: the active preset's name, Update preset, its id as the target", async () => {
		const base = fixture("presets");
		const state = withView(base, {
			overlay: { id: "presetSave", mode: "update", fromRunId: null },
		});
		const { view } = await show(state);
		expect((byRole("textbox", "Name") as HTMLInputElement).value).toBe(
			"Nordwind Logistik GmbH",
		);
		expect(inPortal("dialog").textContent).toContain(
			"Update Nordwind Logistik GmbH",
		);
		await click(byRole("button", "Update preset"));
		const [draft] = view.argsOf("savePreset")[0] ?? [];
		expect(draft?.replaceId).toBe("preset-nordwind");
	});

	test("Cancel closes without saving", async () => {
		const { view } = await show(open());
		await click(byRole("button", "Cancel"));
		expect(view.argsOf("closeOverlay")).toHaveLength(1);
		expect(view.argsOf("savePreset")).toEqual([]);
	});

	test("from a run's menu the run's copy is what is listed", async () => {
		const base = fixture("done");
		const state = withView(base, {
			overlay: { id: "presetSave", mode: "save", fromRunId: "run-11" },
		});
		const { view } = await show(state);
		const name = byRole("textbox", "Name") as HTMLInputElement;
		expect(name.value).toBe("Alpenfracht AG");
		await click(byRole("button", "Save preset"));
		const [draft] = view.argsOf("savePreset")[0] ?? [];
		expect(draft?.fromRunId).toBe("run-11");
		expect(draft?.ticked).toContain("vendor_name");
	});

	test("on a touch screen it is a full-height sheet with the same body", async () => {
		await show(fixture("preset-save"), { ...PHONE_LAYOUT });
		const sheet = inPortal("dialog");
		expect(sheet.textContent).toContain("Save inputs as a preset");
		expect(byRole("button", "Save preset")).toBeTruthy();
		expect(allByRole("textbox", "Name")).toHaveLength(1);
	});
});

describe("the narrow Inputs pane (M1, phone)", () => {
	test("the after-run row ends the pane and opens the Per run sheet", async () => {
		const { view } = await show(fixture("series"), PHONE_LAYOUT);
		expect(byText("3 inputs are per run")).toBeTruthy();
		await click(byRole("button", "Change which inputs are per run"));
		expect(view.argsOf("openOverlay")).toEqual([
			[{ id: "afterRun", focusName: null }],
		]);
	});

	test("one input reads in the singular; none says so once introduced", async () => {
		const small = fixture("small-reset");
		await show(small, PHONE_LAYOUT);
		expect(byText("2 inputs are per run")).toBeTruthy();
		await dom.cleanup();
		const one = {
			...small,
			memory: {
				...small.memory,
				prefs: { ...small.memory.prefs, perRun: ["order"] },
			},
		};
		await show(one, PHONE_LAYOUT);
		expect(byText("1 input is per run")).toBeTruthy();
		await dom.cleanup();
		const none = {
			...small,
			memory: {
				...small.memory,
				prefs: { ...small.memory.prefs, perRun: [], introduced: true },
			},
		};
		await show(none, PHONE_LAYOUT);
		expect(byText("No input is per run")).toBeTruthy();
	});

	test("never before the setting was introduced, never beside a stage", async () => {
		await show(fixture("done"), PHONE_LAYOUT);
		expect(queryByRole("button", /Change which inputs/)).toBeNull();
		await dom.cleanup();
		await show(fixture("series"), DESKTOP_LAYOUT);
		expect(queryByRole("button", /Change which inputs/)).toBeNull();
	});

	test("the name is large and the description whole", async () => {
		await show(fixture("done"), PHONE_LAYOUT);
		const description = byText(
			/^Upload an invoice and its supporting documents/,
		);
		expect(description.className).not.toContain("line-clamp-2");
		expect(byRole("heading", "Extract invoice").className).toContain("text-xl");
	});

	const placesOf = (container: HTMLElement, parts: readonly Element[]) => {
		const line = container.querySelector<HTMLElement>("[data-fw-head-line]");
		if (!line) throw new Error("no head line");
		return parts.map((element) => {
			const item = Array.from(line.children).find((child) =>
				child.contains(element),
			);
			return item?.className.match(/(?:^| )order-(\d)/)?.[1] ?? "none";
		});
	};

	test("the Presets button sits on the name's line, the description right under the name, Reset after it", async () => {
		const state = withView(fixture("presets"), { overlay: null });
		const { container } = await show(state, PHONE_LAYOUT);
		const parts = [
			byRole("heading", state.form.name),
			byRole("button", /Open presets/),
			byText(state.form.description),
			byRole("button", "Reset to defaults"),
		];
		expect(placesOf(container, parts)).toEqual(["1", "3", "4", "5"]);
		expect(container.querySelector("[data-fw-head-line]")?.className).toContain(
			"items-center",
		);
	});

	test("without the button Reset keeps its place at the end of the name's line", async () => {
		const state = fixture("series");
		const { container } = await show(state, PHONE_LAYOUT);
		const parts = [
			byRole("heading", state.form.name),
			byRole("button", "Reset to defaults"),
			byText(state.form.description),
		];
		expect(placesOf(container, parts)).toEqual(["1", "2", "4"]);
	});
});

describe("the rail sizes to its fields (a short form's dock follows them)", () => {
	const listOf = (container: HTMLElement) =>
		container.querySelector("form")?.parentElement as HTMLElement;

	test("the rail and its list take their content's height; the Runs list fills the column", async () => {
		const inputs = await show(fixture("small"));
		const rail = inputs.container.querySelector<HTMLElement>("[data-fw-tab]");
		expect(rail?.dataset.fwTab).toBe("inputs");
		expect(rail?.className).toContain("flex-[0_1_auto]");
		expect(listOf(inputs.container).className).toContain("flex-[0_1_auto]");
		expect(listOf(inputs.container).hasAttribute("data-fw-over")).toBe(false);
		await inputs.unmount();

		const runs = await show(withRail(fixture("runs"), { tab: "runs" }));
		const shown = runs.container.querySelector<HTMLElement>("[data-fw-tab]");
		expect(shown?.dataset.fwTab).toBe("runs");
		expect(shown?.className).toContain("flex-1");
	});

	test("a list taller than its box says so, for the head's hairline and the dock's fill", async () => {
		const prototype = dom.window.HTMLDivElement.prototype;
		const measures = { scrollHeight: 1200, clientHeight: 640 };
		for (const [name, value] of Object.entries(measures))
			Object.defineProperty(prototype, name, {
				configurable: true,
				get: () => value,
			});
		try {
			const { container } = await show(fixture("large"));
			const list = listOf(container);
			expect(list.hasAttribute("data-fw-over")).toBe(true);
			expect(list.className).toContain("border-hairline");
		} finally {
			for (const name of Object.keys(measures))
				Reflect.deleteProperty(prototype, name);
		}
	});
});

describe("the scroll position", () => {
	test("the field list finds its place again after a remount", async () => {
		const first = await show(fixture("idle"));
		const list = first.container.querySelector("form")
			?.parentElement as HTMLElement;
		list.scrollTop = 140;
		await keyDown(list, "x");
		list.dispatchEvent(
			new dom.window.Event("scroll", { bubbles: true }) as unknown as Event,
		);
		await settle();
		await first.unmount();
		const second = await show(fixture("idle"));
		const again = second.container.querySelector("form")
			?.parentElement as HTMLElement;
		expect(again.scrollTop).toBe(140);
	});
});
