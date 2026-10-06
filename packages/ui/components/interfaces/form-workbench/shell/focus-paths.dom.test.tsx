import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type {
	FormSessionState,
	RunOutcome,
	WorkbenchLayout,
} from "../contracts";
import { click, installWorkbenchDom, keyDown, settle } from "../testing/dom";
import { type FixtureName, fixture, fixtureClock } from "../testing/fixtures";
import { DESKTOP_LAYOUT, PHONE_LAYOUT } from "../testing/layouts";
import type { LiveShell } from "./live-test-kit";

/*
 * Focus never falls to the page (PLAN §7), end to end: the real rail, dock, stage and shell on the real
 * session reducer. Each path the reviewers found once left focus on <body> or the interface root.
 */

const dom = installWorkbenchDom();
const { mountLiveShell } = await import("./live-test-kit");

afterEach(dom.cleanup);
afterAll(dom.restore);

const active = () => document.activeElement;

/** An element in words, so a failed match never prints the DOM: its hook, else its tag, and its name. */
function described(element: Element | null) {
	if (!element) return "nothing";
	if (element === document.body) return "the page";
	if (element.hasAttribute("data-fw-root")) return "the interface root";
	const hookValue = element.getAttribute("data-fw-focus");
	const name = element.getAttribute("aria-label") ?? element.textContent ?? "";
	return `${hookValue ?? element.tagName.toLowerCase()} · ${name.trim()}`;
}

const focused = () => described(active());

/** Focus is on exactly this element. */
function expectFocusOn(element: Element | null) {
	expect(focused()).toBe(described(element));
	expect(element !== null && active() === element).toBe(true);
}

/** The rescue waits a frame and a task; a closing popover hands focus back from a task of its own. */
async function settleFocus() {
	for (let step = 0; step < 3; step++) await settle();
}

async function focusOn(element: Element | null) {
	if (!(element instanceof HTMLElement)) throw new Error("nothing to focus");
	await act(async () => element.focus());
	return element;
}

const CARET = 8;

/** A person typing in the middle of a text field: focus with the caret inside the value. */
async function caretIn(element: Element | null) {
	const field = (await focusOn(element)) as HTMLTextAreaElement;
	await act(async () => field.setSelectionRange(CARET, CARET));
	return field;
}

/** Focus is back in the field with the caret where it was, not a select-all. */
function expectCaret(field: HTMLTextAreaElement) {
	expectFocusOn(field);
	expect([field.selectionStart, field.selectionEnd]).toEqual([CARET, CARET]);
}

async function live(
	name: FixtureName,
	state: FormSessionState = fixture(name),
	layout: WorkbenchLayout = DESKTOP_LAYOUT,
) {
	return mountLiveShell(state, layout, fixtureClock(name));
}

const hook = (value: string) => `[data-fw-focus="${value}"]`;
const inStage = (value: string) => `[data-fw-region="stage"] ${hook(value)}`;

function newestRun(view: LiveShell) {
	const run = view.state().runs[0];
	if (!run) throw new Error("no run");
	return run;
}

/** The runtime reports a run under way, the streaming fixture's answer, then its end. */
async function runToEnd(view: LiveShell, runId: string, outcome: RunOutcome) {
	const output = fixture("streaming").runs[0]?.output;
	if (!output) throw new Error("the streaming fixture has no output");
	await view.report({ type: "runAccepted", runId, backendRunId: runId });
	await view.report({ type: "runOutput", runId, output });
	await view.report({ type: "runSettled", runId, outcome });
}

describe("the cursor on open (S6)", () => {
	test("a first field that holds a value gets the caret after it, nothing selected", async () => {
		const view = await live("small-offer");
		const field = active() as HTMLTextAreaElement;
		expect(focused()).toStartWith("field:");
		expect(field.value).not.toBe("");
		const end = field.value.length;
		expect([field.selectionStart, field.selectionEnd]).toEqual([end, end]);
		expect(view.root.hasAttribute("data-fw-quiet-focus")).toBe(true);
		await view.unmount();
	});
});

describe("the cursor a fixture opens with is one the session leaves (spec §8)", () => {
	const dateHook = `input${hook("field:invoice_date")}`;

	test("series: on a split box the date has the cursor, on a phone it is only scrolled to", async () => {
		const desktop = await live("series");
		expectFocusOn(desktop.root.querySelector(dateHook));
		await desktop.unmount();

		const phone = await live("series", fixture("series"), PHONE_LAYOUT);
		const date = phone.root.querySelector(dateHook);
		expect(date).not.toBeNull();
		expect(active()).not.toBe(date);
		expect(phone.root.contains(active())).toBe(false);
		expect(phone.state().view.focus).toBeNull();
		await phone.unmount();
	});

	test("series-failed: “30” is typed, the caret after it, nothing selected", async () => {
		const view = await live("series-failed");
		const date = view.root.querySelector<HTMLInputElement>(dateHook);
		expectFocusOn(date);
		expect(date?.value).toBe("30");
		expect([date?.selectionStart, date?.selectionEnd]).toEqual([2, 2]);
		await view.unmount();
	});
});

describe("a run that ends while its Stop has focus", () => {
	test("Run again puts the cursor on the new run's Stop; when that run ends, on Copy answer", async () => {
		const view = await live("stopped");
		await click(view.root.querySelector(inStage("run-again")) as HTMLElement);
		const started = newestRun(view);
		expect(started.n).toBe(15);
		await settleFocus();
		expect(focused()).toStartWith("stop · ");

		await runToEnd(view, started.id, { kind: "succeeded" });
		await settleFocus();
		expect(newestRun(view).status).toBe("done");
		expectFocusOn(view.root.querySelector(inStage("copy")));
		expect(focused()).toContain("Copy answer");
		await view.unmount();
	});

	test("the run bar's Stop: once the run has stopped, the cursor is on Run again", async () => {
		const view = await live("streaming");
		const stop = await focusOn(view.root.querySelector(inStage("stop")));
		await click(stop);
		const run = newestRun(view);
		expect(run.stopRequested).toBe(true);
		await view.report({
			type: "runSettled",
			runId: run.id,
			outcome: { kind: "stopped" },
		});
		await settleFocus();
		expect(newestRun(view).status).toBe("stopped");
		expectFocusOn(view.root.querySelector(inStage("run-again")));
		await view.unmount();
	});

	test("focus the person moved out of the form stays where they put it", async () => {
		const view = await live("streaming");
		await focusOn(view.root.querySelector(inStage("stop")));
		const outside = document.createElement("button");
		document.body.append(outside);
		await focusOn(outside);
		await view.report({
			type: "runSettled",
			runId: newestRun(view).id,
			outcome: { kind: "succeeded" },
		});
		await settleFocus();
		expectFocusOn(outside);
		outside.remove();
		await view.unmount();
	});
});

describe("Undo after a file removal", () => {
	test("⌘Z puts the file back and keeps the cursor on its row", async () => {
		const view = await live("done");
		const row = await focusOn(
			view.root.querySelector(hook("field:invoice_file")),
		);
		const name = row.getAttribute("aria-label");
		await keyDown(row, "Backspace");
		await settleFocus();
		expect(view.state().undo?.kind).toBe("fileRemoved");
		expect(active()?.getAttribute("data-fw-focus")).toBe("field:invoice_file");
		expect(active()?.getAttribute("aria-label")).not.toBe(name);

		await keyDown(active() as HTMLElement, "z", { metaKey: true });
		await settleFocus();
		expect(view.state().undo).toBeNull();
		expect(active()?.getAttribute("data-fw-focus")).toBe("field:invoice_file");
		expect(active()?.getAttribute("aria-label")).toBe(name);
		await view.unmount();
	});
});

describe("the cursor moving on after a run", () => {
	test("leaves the per-run date it came from without flagging it", async () => {
		const done = fixture("done");
		const state: FormSessionState = {
			...done,
			view: { ...done.view, pane: "inputs" },
			memory: {
				...done.memory,
				prefs: {
					...done.memory.prefs,
					perRun: ["invoice_file", "invoice_date"],
				},
			},
		};
		const view = await live("done", state);
		const date = await focusOn(
			view.root.querySelector(`input${hook("field:invoice_date")}`),
		);
		await keyDown(date, "Enter", { metaKey: true });
		await settleFocus();
		expect(view.state().runs[0]?.n).toBe(15);
		expect(view.state().rail.values.invoice_date).toBe("");
		expect(focused()).toStartWith("field:invoice_file");
		expect(view.state().rail.problems.invoice_date).toBeUndefined();
		expect(view.root.textContent).not.toContain("Choose a date.");
		await view.unmount();
	});
});

describe("presets with none saved yet", () => {
	test("⌘P from a field opens the menu; Esc gives the field its cursor back", async () => {
		const view = await live("done");
		expect(view.state().memory.presets).toHaveLength(0);
		expect(view.root.querySelector(hook("preset-button"))).toBeNull();
		const vendor = await caretIn(
			view.root.querySelector(hook("field:vendor_name")),
		);
		await keyDown(vendor, "p", { metaKey: true });
		await settleFocus();
		expect(view.state().view.overlay?.id).toBe("presets");
		expect(vendor.contains(active())).toBe(false);

		await keyDown(active() as HTMLElement, "Escape");
		await settleFocus();
		expect(view.state().view.overlay).toBeNull();
		expectCaret(vendor);
		await view.unmount();
	});

	test("the menu's Save these inputs as a preset… opens the dialog; closing it gives the field its cursor back", async () => {
		const view = await live("done");
		const vendor = await caretIn(
			view.root.querySelector(hook("field:vendor_name")),
		);
		await keyDown(vendor, "p", { metaKey: true });
		await settleFocus();
		const save = active();
		expect(save?.textContent).toContain("Save these inputs as a preset");
		await click(save as HTMLElement);
		await settleFocus();
		expect(view.state().view.overlay?.id).toBe("presetSave");
		expect(view.root.querySelector("[data-fw-modal]")?.contains(active())).toBe(
			true,
		);

		await keyDown(active() as HTMLElement, "Escape");
		await settleFocus();
		expect(view.state().view.overlay).toBeNull();
		expectCaret(vendor);
		await view.unmount();
	});

	test("on a touch screen the menu is a sheet; closing it gives the field its cursor back", async () => {
		const done = fixture("done");
		const view = await live(
			"done",
			{ ...done, view: { ...done.view, pane: "inputs" } },
			PHONE_LAYOUT,
		);
		expect(view.state().layout?.touch).toBe(true);
		const vendor = await caretIn(
			view.root.querySelector(hook("field:vendor_name")),
		);
		await keyDown(vendor, "p", { metaKey: true });
		await settleFocus();
		const sheet = view.root.querySelector("[data-fw-modal]");
		expect(sheet?.contains(active())).toBe(true);

		await keyDown(active() as HTMLElement, "Escape");
		await settleFocus();
		expect(view.state().view.overlay).toBeNull();
		expectCaret(vendor);
		await view.unmount();
	});
});

describe("the ⌘S dialog", () => {
	async function openFromVendor(view: Awaited<ReturnType<typeof live>>) {
		const vendor = await caretIn(
			view.root.querySelector(hook("field:vendor_name")),
		);
		await keyDown(vendor, "s", { metaKey: true });
		await settleFocus();
		expect(view.state().view.overlay?.id).toBe("presetSave");
		expect(view.root.querySelector("[data-fw-modal]")?.contains(active())).toBe(
			true,
		);
		return vendor;
	}

	test("Esc closes it and the field that had the cursor gets it back", async () => {
		const view = await live("done");
		const vendor = await openFromVendor(view);
		await keyDown(active() as HTMLElement, "Escape");
		await settleFocus();
		expect(view.state().view.overlay).toBeNull();
		expectCaret(vendor);
		await view.unmount();
	});

	test("saving closes it and the field that had the cursor gets it back", async () => {
		const view = await live("done");
		const vendor = await openFromVendor(view);
		const save = Array.from(
			view.root.querySelectorAll<HTMLButtonElement>("[data-fw-modal] button"),
		).find((button) => button.textContent === "Save preset");
		await click(save as HTMLElement);
		await settleFocus();
		expect(view.state().memory.presets).toHaveLength(1);
		expect(view.state().view.overlay).toBeNull();
		expectCaret(vendor);
		await view.unmount();
	});

	describe("on a touch screen it is a full sheet", () => {
		const touchLive = () => {
			const done = fixture("done");
			return live(
				"done",
				{ ...done, view: { ...done.view, pane: "inputs" } },
				PHONE_LAYOUT,
			);
		};

		test("Esc closes it and the field that had the cursor gets it back", async () => {
			const view = await touchLive();
			expect(view.state().layout?.touch).toBe(true);
			const vendor = await openFromVendor(view);
			await keyDown(active() as HTMLElement, "Escape");
			await settleFocus();
			expect(view.state().view.overlay).toBeNull();
			expectCaret(vendor);
			await view.unmount();
		});

		test("saving closes it and the field that had the cursor gets it back", async () => {
			const view = await touchLive();
			const vendor = await openFromVendor(view);
			const save = Array.from(
				view.root.querySelectorAll<HTMLButtonElement>("[data-fw-modal] button"),
			).find((button) => button.textContent === "Save preset");
			await click(save as HTMLElement);
			await settleFocus();
			expect(view.state().memory.presets).toHaveLength(1);
			expect(view.state().view.overlay).toBeNull();
			expectCaret(vendor);
			await view.unmount();
		});
	});
});

describe("Remove from queue", () => {
	test("takes the run out and puts the cursor on Run", async () => {
		const view = await live("queued");
		const selected = view.state().view.selectedRunId;
		const queued = view.state().runs.find((run) => run.id === selected);
		if (queued?.status !== "queued") throw new Error("no queued run on stage");
		const remove = Array.from(
			view.root.querySelectorAll<HTMLButtonElement>(
				'[data-fw-region="stage"] button',
			),
		).find((button) => button.textContent === "Remove from queue");
		if (!remove) throw new Error("no Remove from queue button");
		await focusOn(remove);
		await click(remove);
		await settleFocus();
		expect(view.state().runs.find((run) => run.id === queued.id)?.status).toBe(
			"notStarted",
		);
		expect(active()?.getAttribute("data-fw-focus")).toBe("run");
		await view.unmount();
	});
});
