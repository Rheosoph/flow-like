import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act, useMemo, useState } from "react";
import type {
	DockMessage,
	DockProps,
	FormSessionState,
	ViewState,
} from "../contracts";
import {
	accessibleName,
	allByRole,
	byRole,
	click,
	installWorkbenchDom,
	mountWorkbench,
	queryByRole,
	settle,
} from "../testing/dom";
import { fakeView } from "../testing/fake-actions";
import { type FixtureName, fixture } from "../testing/fixtures";
import { DESKTOP_LAYOUT, layoutFor, withLayout } from "../testing/layouts";

const dom = installWorkbenchDom();
const { Dock } = await import("./dock");

afterEach(dom.cleanup);
afterAll(dom.restore);

async function mountDock(
	state: FormSessionState,
	variant: DockProps["variant"] = "rail",
	layout = DESKTOP_LAYOUT,
) {
	const fake = fakeView(withLayout(state, layout), { layout });
	const view = await mountWorkbench(
		<div data-fw-root="">
			<Dock {...fake.props} variant={variant} />
		</div>,
		{ layout },
	);
	return { view, fake };
}

/** A dock whose overlay opens and closes for real, with a field the cursor can return to. */
function OverlayHarness({ initial }: Readonly<{ initial: FormSessionState }>) {
	const [state, setState] = useState(initial);
	const fake = useMemo(
		() => fakeView(withLayout(initial, DESKTOP_LAYOUT)),
		[initial],
	);
	const actions = useMemo(
		() => ({
			...fake.actions,
			openOverlay: (overlay: NonNullable<ViewState["overlay"]>) =>
				setState((now) => ({ ...now, view: { ...now.view, overlay } })),
			closeOverlay: () =>
				setState((now) => ({ ...now, view: { ...now.view, overlay: null } })),
		}),
		[fake],
	);
	return (
		<div data-fw-root="">
			<input data-fw-focus="field:invoice_date" aria-label="Invoice date" />
			<Dock
				state={withLayout(state, DESKTOP_LAYOUT)}
				actions={actions}
				layout={DESKTOP_LAYOUT}
				routes={fake.routes}
				variant="rail"
			/>
		</div>
	);
}

async function pressEscape(target: Element) {
	const KeyboardEventCtor = (
		target.ownerDocument.defaultView as unknown as typeof globalThis
	).KeyboardEvent;
	await act(async () => {
		target.dispatchEvent(
			new KeyboardEventCtor("keydown", {
				key: "Escape",
				bubbles: true,
				cancelable: true,
			}),
		);
	});
}

const lineOf = (root: ParentNode) =>
	root.querySelector<HTMLElement>("[data-fw-dock-line]");

const kindOf = (root: ParentNode) =>
	lineOf(root)?.getAttribute("data-fw-dock-line") ?? null;

const textOf = (root: ParentNode) =>
	lineOf(root)?.querySelector("span")?.textContent ?? "";

const withView = (
	state: FormSessionState,
	patch: Partial<ViewState>,
): FormSessionState => ({ ...state, view: { ...state.view, ...patch } });

const entry = (message: DockMessage, undo = false) => ({
	seq: 99,
	message,
	undo,
	expiresAt: null,
});

describe("the status line, one thing at a time", () => {
	const cases: readonly (readonly [FixtureName, string, string])[] = [
		["idle", "missing", "3 fields to fill in"],
		["invalid", "problems", "2 fields need a look"],
		["running", "compared", "Same inputs as run 14"],
		["queued", "queue", "3 running · 1 queued"],
		["series-failed", "failure", "Run 18 failed at step 2: Run OCR."],
		["hosted-files", "blocked", "Invoice can't be sent from this page."],
		["small-offer", "question", "Make Order number and Receipt per run?"],
	];

	for (const [name, kind, text] of cases) {
		test(`${name}: ${text}`, async () => {
			const { view } = await mountDock(fixture(name));
			expect(kindOf(view.container)).toBe(kind);
			expect(textOf(view.container)).toBe(text);
			await view.unmount();
		});
	}

	test("files of the entry sending come first", async () => {
		const { view } = await mountDock(fixture("uploading"));
		expect(kindOf(view.container)).toBe("sending");
		expect(textOf(view.container)).toMatch(/^Sending \d of \d files$/);
		await view.unmount();
	});

	test("a message outranks an unseen failure; the hold outranks a message", async () => {
		const failed = fixture("series-failed");
		const message = entry({ kind: "fieldReset", label: "Max pages" }, true);
		const { view } = await mountDock(withView(failed, { message }));
		expect(kindOf(view.container)).toBe("message");
		expect(textOf(view.container)).toBe("Max pages reset.");
		await view.rerender(
			<div data-fw-root="">
				<Dock
					{...fakeView(
						{
							...withView(failed, { message }),
							queue: {
								...failed.queue,
								hold: { runs: [18, 19], step: { number: 2, title: "Run OCR" } },
							},
						},
						{ layout: DESKTOP_LAYOUT },
					).props}
					variant="rail"
				/>
			</div>,
		);
		expect(kindOf(view.container)).toBe("hold");
		await view.unmount();
	});

	test("the offer is a question with Yes and No; a message outranks it", async () => {
		const offer = fixture("small-offer");
		const { view, fake } = await mountDock(offer);
		const yes = byRole("button", "Yes");
		await click(yes);
		await click(byRole("button", "No"));
		expect(fake.argsOf("answerQuestion")).toEqual([[true], [false]]);
		await view.rerender(
			<div data-fw-root="">
				<Dock
					{...fakeView(
						withView(offer, {
							message: entry({ kind: "presetSaved", name: "Nordwind" }),
						}),
						{ layout: DESKTOP_LAYOUT },
					).props}
					variant="rail"
				/>
			</div>,
		);
		expect(kindOf(view.container)).toBe("message");
		await view.unmount();
	});

	test("buttons at the right end of the line act", async () => {
		const queued = await mountDock(fixture("queued"));
		await click(byRole("button", "Clear queue"));
		expect(queued.fake.calls.map((call) => call.name)).toContain("clearQueue");
		await queued.view.unmount();
		const failed = await mountDock(fixture("series-failed"));
		await click(byRole("button", "Show run 18"));
		expect(failed.fake.argsOf("selectRun")).toEqual([
			[expect.stringContaining("18"), "show"],
		]);
		await failed.view.unmount();
	});

	test("only a message is mirrored into the live region", async () => {
		const quiet = await mountDock(fixture("idle"));
		expect(quiet.view.container.querySelector("output")?.textContent).toBe("");
		await quiet.view.unmount();
		const message = entry({ kind: "sameInputs", n: 14 });
		const loud = await mountDock(withView(fixture("done"), { message }));
		expect(loud.view.container.querySelector("output")?.textContent).toBe(
			"Same inputs as run 14. ⌘↵ runs it again.",
		);
		await loud.view.unmount();
	});

	test("a failed answer is a critical message, read out", async () => {
		const message = entry({ kind: "answerFailed", n: 15, runId: "run-15" });
		const { view } = await mountDock(withView(fixture("running"), { message }));
		const text = "Your answer to run 15 could not be sent. Try again.";
		expect(kindOf(view.container)).toBe("message");
		expect(textOf(view.container)).toBe(text);
		expect(lineOf(view.container)?.className).toContain("text-critical");
		expect(view.container.querySelector("output")?.textContent).toBe(text);
		await view.unmount();
	});

	test("while the queue is on hold, fields that need a look keep Resume queue", async () => {
		const invalid = fixture("invalid");
		const { view, fake } = await mountDock({
			...invalid,
			queue: {
				...invalid.queue,
				hold: { runs: [18, 19], step: { number: 2, title: "Run OCR" } },
			},
		});
		expect(kindOf(view.container)).toBe("problems");
		await click(byRole("button", "Resume queue"));
		expect(fake.calls.map((call) => call.name)).toContain("resumeQueue");
		await view.unmount();
	});

	test("Undo comes with a message that can be undone", async () => {
		const message = entry({ kind: "fileRemoved", name: "a.pdf" }, true);
		const { view, fake } = await mountDock(
			withView(fixture("done"), { message }),
		);
		await click(byRole("button", "Undo"));
		expect(fake.calls.map((call) => call.name)).toEqual(["undo"]);
		await view.unmount();
	});
});

describe("Run", () => {
	test("a click presses Run from the button; the title names the key", async () => {
		const { view, fake } = await mountDock(fixture("idle"));
		const run = byRole("button", "Run");
		expect(run.getAttribute("title")).toBe("Run · ⌘↵");
		expect(run.getAttribute("aria-keyshortcuts")).toBe("Meta+Enter");
		expect(run.getAttribute("data-fw-focus")).toBe("run");
		await click(run);
		expect(fake.argsOf("run")).toEqual([[{ from: "button" }]]);
		await view.unmount();
	});

	test("with per-run fields the title also names ⇧⌘↵", async () => {
		const { view } = await mountDock(fixture("series"));
		expect(byRole("button", "Run").getAttribute("title")).toBe(
			"Run · ⌘↵. Run and leave the inputs as they are · ⇧⌘↵",
		);
		await view.unmount();
	});

	test("the label follows the event's own submit label", async () => {
		const state = fixture("idle");
		const custom: FormSessionState = {
			...state,
			form: { ...state.form, submitLabel: "Start" },
		};
		const { view } = await mountDock(custom);
		expect(byRole("button", "Start").getAttribute("title")).toBe("Start · ⌘↵");
		await view.unmount();
	});

	test("outside macOS the keys read Ctrl, in the titles, the chip and the shortcut names", async () => {
		const state = fixture("series");
		const windows: FormSessionState = {
			...state,
			form: { ...state.form, viewer: { ...state.form.viewer, mac: false } },
		};
		const { view } = await mountDock(windows);
		const run = byRole("button", /^Run/);
		expect(run.getAttribute("title")).toBe(
			"Run · Ctrl+Enter. Run and leave the inputs as they are · Ctrl+Shift+Enter",
		);
		expect(run.getAttribute("aria-keyshortcuts")).toBe("Control+Enter");
		expect(run.textContent).toBe("RunCtrl");
		expect(byRole("button", "Stop run 17").getAttribute("title")).toBe(
			"Stop run 17 · Ctrl+.",
		);
		expect(queryByRole("button", "Keyboard shortcuts (Ctrl+/)")).not.toBeNull();
		await view.unmount();
	});

	test("a repeated Enter or Space on Run is stopped; the first one is not", async () => {
		const { view, fake } = await mountDock(fixture("idle"));
		const run = byRole("button", "Run");
		const press = (key: string, init: KeyboardEventInit) => {
			const KeyboardEventCtor = (
				run.ownerDocument.defaultView as unknown as typeof globalThis
			).KeyboardEvent;
			const event = new KeyboardEventCtor("keydown", {
				key,
				bubbles: true,
				cancelable: true,
				...init,
			});
			run.dispatchEvent(event);
			return event.defaultPrevented;
		};
		expect(press("Enter", { repeat: true })).toBe(true);
		expect(press(" ", { repeat: true })).toBe(true);
		expect(press("Enter", { repeat: false })).toBe(false);
		expect(press("Enter", { repeat: true, isComposing: true })).toBe(false);
		expect(fake.argsOf("run")).toEqual([]);
		await view.unmount();
	});
});

describe("keyboard order", () => {
	test("Tab reaches Run and Stop first, then the line's buttons, then Change (spec M1)", async () => {
		const { view } = await mountDock(fixture("series-failed"));
		const buttons = Array.from(
			view.container.querySelectorAll<HTMLElement>("[data-fw-dock] button"),
		).map((button) => accessibleName(button));
		expect(buttons).toEqual([
			"Run",
			"Stop run 21",
			"Show run 18",
			"Change which inputs are per run",
			"Keyboard shortcuts (⌘/)",
		]);
		expect(lineOf(view.container)?.className).toContain("order-first");
		await view.unmount();
	});

	test("the offer's Yes and No come after Run", async () => {
		const { view } = await mountDock(fixture("small-offer"));
		const buttons = Array.from(
			view.container.querySelectorAll<HTMLElement>("[data-fw-dock] button"),
		).map((button) => accessibleName(button));
		expect(buttons.slice(0, 2)).toEqual(["Run", "Stop run 3"]);
		expect(buttons.slice(2)).toEqual(["Yes", "No"]);
		await view.unmount();
	});
});

describe("Stop", () => {
	test("shown for the run on the stage while it is live, named by it", async () => {
		const { view } = await mountDock(fixture("running"));
		const stop = byRole("button", "Stop run 14");
		expect(stop.getAttribute("title")).toBe("Stop run 14 · ⌘.");
		expect(stop.getAttribute("data-fw-focus")).toBe("stop");
		await view.unmount();
	});

	test("a click stops that run and hands the cursor to Run", async () => {
		const { view, fake } = await mountDock(fixture("running"));
		await click(byRole("button", "Stop run 14"));
		const [[runId]] = fake.argsOf("stop");
		expect(runId).toContain("14");
		expect(view.container.ownerDocument.activeElement).toBe(
			byRole("button", "Run"),
		);
		await view.unmount();
	});

	test("a queued run on the stage can be taken out with it", async () => {
		const { view } = await mountDock(fixture("queued"));
		expect(queryByRole("button", "Stop run 18")).not.toBeNull();
		await view.unmount();
	});

	test("hidden when the run on the stage is not live", async () => {
		for (const name of ["done", "failed", "stopped", "idle"] as const) {
			const { view } = await mountDock(fixture(name));
			expect(queryByRole("button", /^Stop/)).toBeNull();
			await view.unmount();
		}
	});
});

describe("the arrows of 'need a look' and 'to fill in'", () => {
	async function mountWithFields(state: FormSessionState) {
		const layout = DESKTOP_LAYOUT;
		const fake = fakeView(withLayout(state, layout), { layout });
		const view = await mountWorkbench(
			<div data-fw-root="">
				<input data-fw-focus="field:invoice_file" aria-label="Invoice" />
				<input data-fw-focus="field:vendor_name" aria-label="Vendor" />
				<input data-fw-focus="field:invoice_date" aria-label="Invoice date" />
				<Dock {...fake.props} variant="rail" />
			</div>,
			{ layout },
		);
		return { view, fake };
	}

	const activeLabel = () =>
		document.activeElement?.getAttribute("aria-label") ?? "";

	test("next goes to the first problem, then on, then wraps; previous goes back", async () => {
		const { view } = await mountWithFields(fixture("invalid"));
		const next = byRole("button", "Next field that needs a look");
		const previous = byRole("button", "Previous field that needs a look");
		await click(next);
		expect(activeLabel()).toBe("Invoice");
		await click(next);
		expect(activeLabel()).toBe("Vendor");
		await click(next);
		expect(activeLabel()).toBe("Invoice");
		await click(previous);
		expect(activeLabel()).toBe("Vendor");
		await view.unmount();
	});

	test("previous from nowhere lands on the last one; fields to fill in work the same", async () => {
		const { view } = await mountWithFields(fixture("idle"));
		await click(byRole("button", "Previous field to fill in"));
		expect(activeLabel()).toBe("Invoice date");
		await click(byRole("button", "Next field to fill in"));
		expect(activeLabel()).toBe("Invoice");
		await view.unmount();
	});

	test("a field that is not in the page is revealed first", async () => {
		const { view, fake } = await mountDock(fixture("invalid"));
		await click(byRole("button", "Next field that needs a look"));
		expect(fake.calls.map((call) => call.name)).toEqual([
			"setRailTab",
			"setPane",
			"setFilter",
		]);
		expect(fake.argsOf("setFilter")).toEqual([[{ query: "", chip: "all" }]]);
		await view.unmount();
	});
});

describe("the after-run line", () => {
	test("hidden until the setting is introduced", async () => {
		const { view } = await mountDock(fixture("idle"));
		expect(view.container.querySelector("[data-fw-after-run]")).toBeNull();
		await view.unmount();
	});

	test("3 inputs are per run · Change, with the keyboard icon", async () => {
		const { view, fake } = await mountDock(fixture("series"));
		const line = view.container.querySelector("[data-fw-after-run]");
		expect(line?.textContent).toContain("3 inputs are per run");
		const change = byRole("button", "Change which inputs are per run");
		expect(change.getAttribute("data-fw-focus")).toBe("change");
		await click(byRole("button", "Keyboard shortcuts (⌘/)"));
		expect(fake.argsOf("openOverlay")).toEqual([[{ id: "shortcuts" }]]);
		await view.unmount();
	});

	test("a form of three fields has no keyboard icon", async () => {
		const { view } = await mountDock(fixture("small-reset"));
		const line = view.container.querySelector("[data-fw-after-run]");
		expect(line?.textContent).toContain("2 inputs are per run");
		expect(queryByRole("button", /Keyboard shortcuts/)).toBeNull();
		await view.unmount();
	});

	test("Change opens the Per run panel", async () => {
		const { view, fake } = await mountDock(fixture("series"));
		await click(byRole("button", "Change which inputs are per run"));
		expect(fake.argsOf("openOverlay")).toEqual([
			[{ id: "afterRun", focusName: null }],
		]);
		await view.unmount();
	});

	test("the dock grows only by the line: no line, no extra row", async () => {
		const quiet = await mountDock(fixture("idle"));
		expect(
			quiet.view.container.querySelectorAll("[data-fw-dock] > *"),
		).toHaveLength(3);
		await quiet.view.unmount();
		const loud = await mountDock(fixture("series"));
		expect(
			loud.view.container.querySelectorAll("[data-fw-dock] > *"),
		).toHaveLength(4);
		await loud.view.unmount();
	});
});

describe("the Per run popover", () => {
	test("spec §8.7: a row per field, checked and what it goes back to", async () => {
		const { view } = await mountDock(fixture("after-run"));
		await settle();
		const dialog = byRole("dialog", "Per run");
		const boxes = allByRole("checkbox", undefined, dialog);
		expect(boxes).toHaveLength(9);
		const row = (name: string) =>
			dialog.querySelector<HTMLInputElement>(`input[data-per-run="${name}"]`);
		expect(row("invoice_file")?.checked).toBe(true);
		expect(row("vendor_name")?.checked).toBe(false);
		const backOf = (name: string) =>
			row(name)?.closest("label")?.querySelectorAll("span")[2]?.textContent;
		expect(backOf("invoice_file")).toBe("Next file");
		expect(backOf("supporting_documents")).toBe("Empty");
		expect(backOf("max_pages")).toBe("Back to 20");
		expect(backOf("run_ocr")).toBe("Back to On");
		expect(backOf("cost_centers")).toBe("Back to 4400, 4410");
		await view.unmount();
	});

	test("the first row takes the cursor; Space toggles a row", async () => {
		const { view, fake } = await mountDock(fixture("after-run"));
		await settle();
		const dialog = byRole("dialog", "Per run");
		const first = dialog.querySelector<HTMLInputElement>("input[data-per-run]");
		expect(document.activeElement).toBe(first);
		const vendor = dialog.querySelector<HTMLInputElement>(
			'input[data-per-run="vendor_name"]',
		);
		if (!vendor) throw new Error("no Vendor row");
		await click(vendor);
		expect(fake.argsOf("setPerRun")).toEqual([["vendor_name", true]]);
		await view.unmount();
	});

	test("↑ / ↓ move the cursor between the rows and stop at both ends (spec §4)", async () => {
		const { view, fake } = await mountDock(fixture("after-run"));
		await settle();
		const dialog = byRole("dialog", "Per run");
		const rows = Array.from(
			dialog.querySelectorAll<HTMLInputElement>("input[data-per-run]"),
		);
		const KeyboardEventCtor = (
			dialog.ownerDocument.defaultView as unknown as typeof globalThis
		).KeyboardEvent;
		const press = async (key: string, shiftKey = false) => {
			const target = document.activeElement as HTMLElement;
			const event = new KeyboardEventCtor("keydown", {
				key,
				shiftKey,
				bubbles: true,
				cancelable: true,
			});
			await act(async () => {
				target.dispatchEvent(event);
			});
			return event.defaultPrevented;
		};
		const focused = () => rows.indexOf(document.activeElement as never);
		expect(focused()).toBe(0);
		expect(await press("ArrowUp")).toBe(true);
		expect(focused()).toBe(0);
		await press("ArrowDown");
		await press("ArrowDown");
		expect(focused()).toBe(2);
		await press("ArrowUp");
		expect(focused()).toBe(1);
		expect(await press("ArrowDown", true)).toBe(false);
		expect(focused()).toBe(1);
		for (const _ of rows) await press("ArrowDown");
		expect(focused()).toBe(rows.length - 1);
		expect(fake.argsOf("setPerRun")).toEqual([]);
		await view.unmount();
	});

	test("Uncheck all and Done", async () => {
		const { view, fake } = await mountDock(fixture("after-run"));
		await settle();
		await click(byRole("button", "Uncheck all"));
		expect(fake.calls.map((call) => call.name)).toContain("uncheckAllPerRun");
		await click(byRole("button", "Done"));
		expect(fake.calls.map((call) => call.name)).toContain("closeOverlay");
		await view.unmount();
	});

	test("Uncheck all is aria-disabled while nothing is checked, and does nothing", async () => {
		const state = fixture("after-run");
		const nothing: FormSessionState = {
			...state,
			memory: {
				...state.memory,
				prefs: { ...state.memory.prefs, perRun: [], auto: [] },
			},
			rail: { ...state.rail, nextFiles: {} },
		};
		const { view, fake } = await mountDock(nothing);
		await settle();
		const uncheck = byRole("button", "Uncheck all");
		expect(uncheck.getAttribute("aria-disabled")).toBe("true");
		await click(uncheck);
		expect(fake.calls.map((call) => call.name)).not.toContain(
			"uncheckAllPerRun",
		);
		const make = byRole("button", "Make files and dates per run");
		await click(make);
		expect(fake.calls.map((call) => call.name)).toContain(
			"perRunFilesAndDates",
		);
		await view.unmount();
	});

	test("Esc closes the panel", async () => {
		const { view, fake } = await mountDock(fixture("after-run"));
		await settle();
		const dialog = byRole("dialog", "Per run");
		const KeyboardEventCtor = (
			dialog.ownerDocument.defaultView as unknown as typeof globalThis
		).KeyboardEvent;
		dialog.dispatchEvent(
			new KeyboardEventCtor("keydown", {
				key: "Escape",
				bubbles: true,
				cancelable: true,
			}),
		);
		await settle();
		expect(fake.calls.map((call) => call.name)).toContain("closeOverlay");
		await view.unmount();
	});

	test("Esc gives the cursor back to Change", async () => {
		const view = await mountWorkbench(
			<OverlayHarness
				initial={withView(fixture("series"), { message: null })}
			/>,
			{ layout: DESKTOP_LAYOUT },
		);
		const change = byRole("button", "Change which inputs are per run");
		await click(change);
		await settle();
		const dialog = byRole("dialog", "Per run");
		await pressEscape(dialog);
		await settle();
		expect(queryByRole("dialog", "Per run")).toBeNull();
		expect(document.activeElement).toBe(change);
		await view.unmount();
	});

	test("opened from a field's marker, Esc gives the cursor back to that field", async () => {
		const opened = withView(fixture("series"), {
			message: null,
			overlay: { id: "afterRun", focusName: "invoice_date" },
		});
		const view = await mountWorkbench(<OverlayHarness initial={opened} />, {
			layout: DESKTOP_LAYOUT,
		});
		await settle();
		await pressEscape(byRole("dialog", "Per run"));
		await settle();
		expect(queryByRole("dialog", "Per run")).toBeNull();
		expect(document.activeElement?.getAttribute("data-fw-focus")).toBe(
			"field:invoice_date",
		);
		await view.unmount();
	});

	test("a field marker's row takes the cursor", async () => {
		const state = fixture("after-run");
		const opened = withView(state, {
			overlay: { id: "afterRun", focusName: "invoice_date" },
		});
		const { view } = await mountDock(opened);
		await settle();
		expect(
			(document.activeElement as HTMLInputElement | null)?.dataset.perRun,
		).toBe("invoice_date");
		await view.unmount();
	});

	test("from 12 fields the panel has a filter that narrows the rows", async () => {
		const large = fixture("large");
		const opened: FormSessionState = {
			...large,
			memory: {
				...large.memory,
				prefs: { ...large.memory.prefs, introduced: true },
			},
			view: { ...large.view, overlay: { id: "afterRun", focusName: null } },
		};
		const { view } = await mountDock(opened);
		await settle();
		const dialog = byRole("dialog", "Per run");
		const filter = byRole("textbox", "Filter fields", dialog);
		const before = allByRole("checkbox", undefined, dialog).length;
		expect(before).toBeGreaterThanOrEqual(12);
		const { typeInto } = await import("../testing/dom");
		await typeInto(filter, "customer");
		const after = allByRole("checkbox", undefined, dialog).length;
		expect(after).toBeGreaterThan(0);
		expect(after).toBeLessThan(before);
		await view.unmount();
	});

	test("a touch screen in a wide box keeps the popover with 44 px rows", async () => {
		const touch = layoutFor(1200, 800, { touch: true });
		const { view } = await mountDock(fixture("after-run"), "rail", touch);
		await settle();
		const dialog = byRole("dialog", "Per run");
		const label = dialog.querySelector("label");
		expect(label?.className).toContain("min-h-11");
		await view.unmount();
	});
});
