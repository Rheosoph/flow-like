import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type { FormSessionState } from "../contracts";
import { installWorkbenchDom, mountWorkbench } from "../testing/dom";
import { fixture } from "../testing/fixtures";
import { DESKTOP_LAYOUT } from "../testing/layouts";
import { shellKit } from "./shell-test-kit";

const dom = installWorkbenchDom();
const { ShellFrame } = await import("./shell-frame");

afterEach(dom.cleanup);
afterAll(dom.restore);

async function mount(state: FormSessionState) {
	const kit = shellKit(state);
	const view = await mountWorkbench(<ShellFrame {...kit.props} />, {
		layout: DESKTOP_LAYOUT,
	});
	const root = view.container.querySelector("[data-fw-root]") as HTMLElement;
	const pick = <T extends HTMLElement>(selector: string) =>
		root.querySelector(selector) as T;
	return { ...kit, view, root, pick };
}

interface KeyInit {
	readonly meta?: boolean;
	readonly ctrl?: boolean;
	readonly shift?: boolean;
	readonly alt?: boolean;
	readonly repeat?: boolean;
	readonly composing?: boolean;
	readonly keyCode?: number;
}

const KEY_DEFAULTS = {
	meta: false,
	ctrl: false,
	shift: false,
	alt: false,
	repeat: false,
	composing: false,
	keyCode: 0,
} as const;

/** A keydown the test can look at afterwards: the harness's `keyDown` hides the event. */
async function press(target: Element, key: string, init: KeyInit = {}) {
	const view = target.ownerDocument.defaultView as unknown as typeof globalThis;
	const given = { ...KEY_DEFAULTS, ...init };
	const event = new view.KeyboardEvent("keydown", {
		key,
		metaKey: given.meta,
		ctrlKey: given.ctrl,
		shiftKey: given.shift,
		altKey: given.alt,
		repeat: given.repeat,
		isComposing: given.composing,
		keyCode: given.keyCode,
		bubbles: true,
		cancelable: true,
	});
	await act(async () => {
		target.dispatchEvent(event);
	});
	return event;
}

const active = () => document.activeElement;

describe("⌘↵ and ⇧⌘↵", () => {
	test("run from inside a text field and keep the keystroke from the field", async () => {
		const { fake, pick } = await mount(fixture("idle"));
		const event = await press(pick("[aria-label=Vendor]"), "Enter", {
			meta: true,
		});
		expect(event.defaultPrevented).toBe(true);
		expect(fake.argsOf("run")).toEqual([[{ leaveAsIs: false, from: "chord" }]]);
	});

	test("⇧⌘↵ runs and leaves the inputs as they are", async () => {
		const { fake, pick } = await mount(fixture("idle"));
		await press(pick("[aria-label=Vendor]"), "Enter", {
			meta: true,
			shift: true,
		});
		expect(fake.argsOf("run")).toEqual([[{ leaveAsIs: true, from: "chord" }]]);
	});

	test("run from a button and from the stage", async () => {
		const { fake, pick } = await mount(fixture("done"));
		await press(pick("[data-fw-focus=copy]"), "Enter", { meta: true });
		await press(pick("[data-fw-focus=run]"), "Enter", { meta: true });
		expect(fake.argsOf("run")).toHaveLength(2);
	});

	test("Ctrl+Enter outside macOS, and ⌘↵ is then not a chord", async () => {
		const state = fixture("idle");
		const { fake, pick } = await mount({
			...state,
			form: { ...state.form, viewer: { ...state.form.viewer, mac: false } },
		});
		await press(pick("[aria-label=Vendor]"), "Enter", { meta: true });
		expect(fake.calls).toEqual([]);
		await press(pick("[aria-label=Vendor]"), "Enter", { ctrl: true });
		expect(fake.argsOf("run")).toHaveLength(1);
	});

	test("a held chord runs once", async () => {
		const { fake, pick } = await mount(fixture("idle"));
		const input = pick("[aria-label=Vendor]");
		await press(input, "Enter", { meta: true });
		const held = await press(input, "Enter", { meta: true, repeat: true });
		expect(held.defaultPrevented).toBe(true);
		expect(fake.argsOf("run")).toHaveLength(1);
	});

	test("an open IME composition owns the key", async () => {
		const { fake, pick } = await mount(fixture("idle"));
		const input = pick("[aria-label=Vendor]");
		const flagged = await press(input, "Enter", {
			meta: true,
			composing: true,
		});
		const coded = await press(input, "Enter", { meta: true, keyCode: 229 });
		expect(flagged.defaultPrevented).toBe(false);
		expect(coded.defaultPrevented).toBe(false);
		expect(fake.calls).toEqual([]);
	});
});

describe("a held Enter or Space on a button", () => {
	test("does not click it again and again", async () => {
		const { fake, pick } = await mount(fixture("idle"));
		const run = pick("[data-fw-focus=run]");
		const first = await press(run, "Enter");
		const held = await press(run, "Enter", { repeat: true });
		const heldSpace = await press(run, " ", { repeat: true });
		expect(first.defaultPrevented).toBe(false);
		expect(held.defaultPrevented).toBe(true);
		expect(heldSpace.defaultPrevented).toBe(true);
		expect(fake.calls).toEqual([]);
	});

	test("leaves a held Enter in a text field alone", async () => {
		const { pick } = await mount(fixture("idle"));
		const held = await press(pick("[aria-label=Vendor]"), "Enter", {
			repeat: true,
		});
		expect(held.defaultPrevented).toBe(false);
	});
});

describe("⌘. and ⌘Z", () => {
	test("⌘. stops the run on the stage", async () => {
		const state = fixture("running");
		const { fake, pick } = await mount(state);
		await press(pick("[data-fw-focus=run]"), ".", { meta: true });
		expect(fake.argsOf("stop")).toHaveLength(1);
	});

	test("⌘. with an ended run on the stage does nothing", async () => {
		const { fake, pick } = await mount(fixture("done"));
		const event = await press(pick("[data-fw-focus=run]"), ".", { meta: true });
		expect(event.defaultPrevented).toBe(false);
		expect(fake.calls).toEqual([]);
	});

	test("⌘Z undoes outside a text field and is the field's own in one", async () => {
		const base = fixture("done");
		const { fake, pick } = await mount({
			...base,
			undo: {
				seq: 9,
				kind: "fieldReset",
				values: base.rail.values,
				nextFiles: {},
				activePresetId: null,
				deletedPreset: null,
			},
		});
		const inField = await press(pick("[aria-label=Vendor]"), "z", {
			meta: true,
		});
		expect(inField.defaultPrevented).toBe(false);
		expect(fake.calls).toEqual([]);
		const onButton = await press(pick("[data-fw-focus=run]"), "z", {
			meta: true,
		});
		expect(onButton.defaultPrevented).toBe(true);
		expect(fake.calls).toEqual([{ name: "undo", args: [] }]);
	});
});

describe("⌘/ and ?", () => {
	test("⌘/ opens the shortcuts sheet from a text field", async () => {
		const { fake, pick } = await mount(fixture("idle"));
		await press(pick("[aria-label=Vendor]"), "/", { meta: true });
		expect(fake.argsOf("openOverlay")).toEqual([[{ id: "shortcuts" }]]);
	});

	test("? opens it from a button, and types a question mark in a field", async () => {
		const { fake, pick } = await mount(fixture("idle"));
		const typed = await press(pick("[aria-label=Vendor]"), "?", {
			shift: true,
		});
		expect(typed.defaultPrevented).toBe(false);
		expect(fake.calls).toEqual([]);
		await press(pick("[data-fw-focus=run]"), "?", { shift: true });
		expect(fake.argsOf("openOverlay")).toEqual([[{ id: "shortcuts" }]]);
	});
});

describe("⌘P and ⌘S", () => {
	test("⌘S opens the save dialog in save mode, ⌘P the presets menu, in any form with fields", async () => {
		const idle = await mount(fixture("idle"));
		await press(idle.pick("[aria-label=Vendor]"), "s", { meta: true });
		await press(idle.pick("[aria-label=Vendor]"), "p", { meta: true });
		expect(idle.fake.argsOf("openOverlay")).toEqual([
			[{ id: "presetSave", mode: "save", fromRunId: null }],
			[{ id: "presets" }],
		]);
	});

	test("with a preset on this device ⌘P opens the menu and ⌘S updates the active preset", async () => {
		const state = fixture("presets");
		const { fake, pick } = await mount({
			...state,
			view: { ...state.view, overlay: null },
		});
		await press(pick("[aria-label=Vendor]"), "p", { meta: true });
		await press(pick("[aria-label=Vendor]"), "s", { meta: true });
		expect(fake.argsOf("openOverlay")).toEqual([
			[{ id: "presets" }],
			[{ id: "presetSave", mode: "update", fromRunId: null }],
		]);
	});
});

describe("/", () => {
	test("focuses Filter fields on a big form when focus is not in a text field", async () => {
		const { pick } = await mount(fixture("large"));
		const presets = pick("[data-fw-focus=preset-button]");
		presets.focus();
		const event = await press(presets, "/");
		expect(event.defaultPrevented).toBe(true);
		expect(active()).toBe(pick("[data-fw-focus=filter]"));
	});

	test("is a slash in a text field and nothing on a smaller form", async () => {
		const big = await mount(fixture("large"));
		const typed = await press(big.pick("[aria-label=Vendor]"), "/");
		expect(typed.defaultPrevented).toBe(false);
		expect(active()).not.toBe(big.pick("[data-fw-focus=filter]"));
		await big.view.unmount();

		const small = await mount(fixture("idle"));
		const event = await press(small.pick("[data-fw-focus=preset-button]"), "/");
		expect(event.defaultPrevented).toBe(false);
	});

	test("with the rail on its Runs tab it switches to Inputs first", async () => {
		const base = fixture("large");
		const { fake, pick } = await mount({
			...base,
			rail: { ...base.rail, tab: "runs" },
		});
		await press(pick("[data-fw-focus=preset-button]"), "/");
		expect(fake.argsOf("setRailTab")).toEqual([["inputs"]]);
	});
});

describe("⌘O", () => {
	function clicks(input: HTMLElement) {
		const seen: Event[] = [];
		input.addEventListener("click", (event) => {
			event.preventDefault();
			seen.push(event);
		});
		return seen;
	}

	test("opens the file dialog of the focused file field", async () => {
		const { pick } = await mount(fixture("idle"));
		const invoice = clicks(pick("[data-fw-file-input=invoice_file]"));
		const supporting = clicks(
			pick("[data-fw-file-input=supporting_documents]"),
		);
		const event = await press(
			pick("[data-fw-focus='field:supporting_documents']"),
			"o",
			{ meta: true },
		);
		expect(event.defaultPrevented).toBe(true);
		expect([invoice.length, supporting.length]).toEqual([0, 1]);
	});

	test("else the first empty one, required first", async () => {
		const { pick } = await mount(fixture("idle"));
		const invoice = clicks(pick("[data-fw-file-input=invoice_file]"));
		const supporting = clicks(
			pick("[data-fw-file-input=supporting_documents]"),
		);
		await press(pick("[aria-label=Vendor]"), "o", { meta: true });
		expect([invoice.length, supporting.length]).toEqual([1, 0]);
	});

	test("a hosted page cannot send the file, so the key is the browser's", async () => {
		const { pick } = await mount(fixture("hosted-files"));
		const invoice = clicks(pick("[data-fw-file-input=invoice_file]"));
		const event = await press(pick("[aria-label=Vendor]"), "o", { meta: true });
		expect(event.defaultPrevented).toBe(false);
		expect(invoice).toHaveLength(0);
	});
});
