import { describe, expect, test } from "bun:test";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import {
	FILE_FIELD_ATTR,
	FILE_INPUT_ATTR,
	type FormSessionState,
} from "../contracts";
import { fakeActions } from "../testing/fake-actions";
import { fixture } from "../testing/fixtures";
import {
	type KeyContext,
	chordOf,
	handleRootKeyDown,
	isComposingKey,
	isTextEntry,
	liveStageRun,
} from "./keyboard";

const bare = { metaKey: false, ctrlKey: false, altKey: false, shiftKey: false };
const cmd = { ...bare, metaKey: true };
const ctrl = { ...bare, ctrlKey: true };

describe("chordOf", () => {
	test("⌘↵ runs, ⇧⌘↵ runs and leaves the inputs, on a Mac", () => {
		expect(chordOf({ key: "Enter", ...cmd }, true)).toBe("run");
		expect(chordOf({ key: "Enter", ...cmd, shiftKey: true }, true)).toBe(
			"runLeave",
		);
		expect(chordOf({ key: "Enter", ...ctrl }, true)).toBeNull();
		expect(chordOf({ key: "Enter", ...cmd, altKey: true }, true)).toBeNull();
		expect(chordOf({ key: "Enter", ...bare }, true)).toBeNull();
	});

	test("Ctrl stands for ⌘ elsewhere, and the Windows key does not", () => {
		expect(chordOf({ key: "Enter", ...ctrl }, false)).toBe("run");
		expect(chordOf({ key: "Enter", ...ctrl, shiftKey: true }, false)).toBe(
			"runLeave",
		);
		expect(chordOf({ key: "Enter", ...cmd }, false)).toBeNull();
		expect(chordOf({ key: "p", ...ctrl }, false)).toBe("presets");
	});

	test("the other ⌘ chords", () => {
		expect(chordOf({ key: ".", ...cmd }, true)).toBe("stop");
		expect(chordOf({ key: "o", ...cmd }, true)).toBe("chooseFiles");
		expect(chordOf({ key: "O", ...cmd }, true)).toBe("chooseFiles");
		expect(chordOf({ key: "p", ...cmd }, true)).toBe("presets");
		expect(chordOf({ key: "s", ...cmd }, true)).toBe("savePreset");
		expect(chordOf({ key: "z", ...cmd }, true)).toBe("undo");
		expect(chordOf({ key: "/", ...cmd }, true)).toBe("shortcuts");
	});

	test("a layout that types / with Shift still opens the shortcuts, redo and other chords stay the browser's", () => {
		expect(chordOf({ key: "/", ...cmd, shiftKey: true }, true)).toBe(
			"shortcuts",
		);
		expect(chordOf({ key: "z", ...cmd, shiftKey: true }, true)).toBeNull();
		expect(chordOf({ key: "p", ...cmd, altKey: true }, true)).toBeNull();
		expect(chordOf({ key: "k", ...cmd }, true)).toBeNull();
	});

	test("? and / are typing keys, with or without Shift, and never with a modifier", () => {
		expect(chordOf({ key: "?", ...bare }, true)).toBe("help");
		expect(chordOf({ key: "?", ...bare, shiftKey: true }, true)).toBe("help");
		expect(chordOf({ key: "/", ...bare }, true)).toBe("filter");
		expect(chordOf({ key: "/", ...bare, shiftKey: true }, true)).toBe("filter");
		expect(chordOf({ key: "/", ...ctrl }, true)).toBeNull();
		expect(chordOf({ key: "/", ...bare, altKey: true }, true)).toBeNull();
		expect(chordOf({ key: "a", ...bare }, true)).toBeNull();
		expect(chordOf({ key: "Enter", ...bare }, true)).toBeNull();
	});
});

function target(tag: string, extra: Record<string, unknown> = {}): EventTarget {
	return {
		tagName: tag,
		getAttribute: () => null,
		closest: () => null,
		...extra,
	} as unknown as EventTarget;
}

describe("isTextEntry", () => {
	test("text controls take typing", () => {
		expect(isTextEntry(target("TEXTAREA"))).toBe(true);
		expect(isTextEntry(target("SELECT"))).toBe(true);
		expect(isTextEntry(target("INPUT", { type: "text" }))).toBe(true);
		expect(isTextEntry(target("INPUT", { type: "search" }))).toBe(true);
		expect(isTextEntry(target("INPUT", { type: "number" }))).toBe(true);
		expect(isTextEntry(target("INPUT", { type: "date" }))).toBe(true);
		expect(isTextEntry(target("INPUT", {}))).toBe(true);
	});

	test("buttons, checkboxes, files and the page do not", () => {
		expect(isTextEntry(target("BUTTON"))).toBe(false);
		expect(isTextEntry(target("DIV"))).toBe(false);
		expect(isTextEntry(target("INPUT", { type: "checkbox" }))).toBe(false);
		expect(isTextEntry(target("INPUT", { type: "file" }))).toBe(false);
		expect(isTextEntry(null)).toBe(false);
		expect(isTextEntry(undefined)).toBe(false);
	});

	test("editable content and text roles count", () => {
		expect(isTextEntry(target("DIV", { isContentEditable: true }))).toBe(true);
		expect(
			isTextEntry(
				target("DIV", {
					getAttribute: (name: string) =>
						name === "contenteditable" ? "true" : null,
				}),
			),
		).toBe(true);
		expect(
			isTextEntry(
				target("DIV", {
					getAttribute: (name: string) => (name === "role" ? "textbox" : null),
				}),
			),
		).toBe(true);
	});
});

describe("isComposingKey", () => {
	test("an open IME composition, by flag or by keyCode 229", () => {
		expect(
			isComposingKey({
				keyCode: 13,
				nativeEvent: { isComposing: true },
			} as unknown as ReactKeyboardEvent),
		).toBe(true);
		expect(
			isComposingKey({
				keyCode: 229,
				nativeEvent: { isComposing: false },
			} as unknown as ReactKeyboardEvent),
		).toBe(true);
		expect(
			isComposingKey({
				keyCode: 13,
				nativeEvent: { isComposing: false },
			} as unknown as ReactKeyboardEvent),
		).toBe(false);
	});
});

describe("liveStageRun", () => {
	test("the run on the stage while it can still be stopped", () => {
		expect(liveStageRun(fixture("running"))?.status).toBe("running");
		expect(liveStageRun(fixture("queued"))?.status).toBe("queued");
	});

	test("nothing to stop on an ended run or an empty stage", () => {
		expect(liveStageRun(fixture("done"))).toBeNull();
		expect(liveStageRun(fixture("idle"))).toBeNull();
	});
});

interface PressInit {
	readonly key: string;
	readonly mods?: Partial<typeof bare>;
	readonly repeat?: boolean;
	readonly composing?: boolean;
	readonly keyCode?: number;
	readonly target?: EventTarget | null;
	readonly prevented?: boolean;
}

const PRESS_DEFAULTS = {
	mods: {},
	repeat: false,
	composing: false,
	keyCode: 0,
	target: null,
	prevented: false,
} as const;

function press(init: PressInit, context: KeyContext) {
	const given = { ...PRESS_DEFAULTS, ...init };
	let prevented: boolean = given.prevented;
	const event = {
		key: given.key,
		...bare,
		...given.mods,
		repeat: given.repeat,
		keyCode: given.keyCode,
		nativeEvent: { isComposing: given.composing },
		target: given.target,
		get defaultPrevented() {
			return prevented;
		},
		preventDefault() {
			prevented = true;
		},
	} as unknown as ReactKeyboardEvent<HTMLElement>;
	handleRootKeyDown(event, context);
	return { prevented };
}

function contextOf(state: FormSessionState, root: HTMLElement | null = null) {
	const fake = fakeActions();
	const focused: string[] = [];
	const context: KeyContext = {
		state,
		actions: fake.actions,
		root,
		focusSoon: (value) => focused.push(value),
	};
	return { context, fake, focused };
}

const withViewer = (
	state: FormSessionState,
	mac: boolean,
): FormSessionState => ({
	...state,
	form: { ...state.form, viewer: { ...state.form.viewer, mac } },
});

describe("handleRootKeyDown · run", () => {
	test("⌘↵ runs and keeps the keystroke from the control", () => {
		const { context, fake } = contextOf(fixture("idle"));
		const { prevented } = press({ key: "Enter", mods: cmd }, context);
		expect(prevented).toBe(true);
		expect(fake.calls).toEqual([
			{ name: "run", args: [{ leaveAsIs: false, from: "chord" }] },
		]);
	});

	test("⇧⌘↵ runs and leaves the inputs, in a form with fields", () => {
		const { context, fake } = contextOf(fixture("idle"));
		press({ key: "Enter", mods: { ...cmd, shiftKey: true } }, context);
		expect(fake.argsOf("run")).toEqual([[{ leaveAsIs: true, from: "chord" }]]);
	});

	test("a form without fields has no ⇧⌘↵ but runs on ⌘↵", () => {
		const { context, fake } = contextOf(fixture("none"));
		const leave = press(
			{ key: "Enter", mods: { ...cmd, shiftKey: true } },
			context,
		);
		expect(leave.prevented).toBe(false);
		expect(fake.calls).toEqual([]);
		press({ key: "Enter", mods: cmd }, context);
		expect(fake.argsOf("run")).toEqual([[{ leaveAsIs: false, from: "chord" }]]);
	});

	test("a held chord runs once: repeats are swallowed but start nothing", () => {
		const { context, fake } = contextOf(fixture("idle"));
		const first = press({ key: "Enter", mods: cmd }, context);
		const held = press({ key: "Enter", mods: cmd, repeat: true }, context);
		expect(first.prevented).toBe(true);
		expect(held.prevented).toBe(true);
		expect(fake.argsOf("run")).toHaveLength(1);
	});

	test("an IME composition owns every key", () => {
		const { context, fake } = contextOf(fixture("idle"));
		const flag = press({ key: "Enter", mods: cmd, composing: true }, context);
		const code = press({ key: "Enter", mods: cmd, keyCode: 229 }, context);
		expect(flag.prevented).toBe(false);
		expect(code.prevented).toBe(false);
		expect(fake.calls).toEqual([]);
	});

	test("a control that already handled the key keeps it", () => {
		const { context, fake } = contextOf(fixture("idle"));
		press({ key: "Enter", mods: cmd, prevented: true }, context);
		expect(fake.calls).toEqual([]);
	});

	test("Ctrl+Enter outside macOS, never ⌘↵", () => {
		const state = withViewer(fixture("idle"), false);
		const { context, fake } = contextOf(state);
		press({ key: "Enter", mods: cmd }, context);
		expect(fake.calls).toEqual([]);
		press({ key: "Enter", mods: ctrl }, context);
		expect(fake.argsOf("run")).toHaveLength(1);
	});

	test("the chord runs from inside a text field", () => {
		const { context, fake } = contextOf(fixture("idle"));
		press({ key: "Enter", mods: cmd, target: target("TEXTAREA") }, context);
		expect(fake.argsOf("run")).toHaveLength(1);
	});
});

describe("handleRootKeyDown · a held Enter or Space on a button", () => {
	const button = target("BUTTON", { closest: () => ({}) });

	test("is swallowed, so a held key does not click again and again", () => {
		const { context, fake } = contextOf(fixture("idle"));
		expect(
			press({ key: "Enter", repeat: true, target: button }, context).prevented,
		).toBe(true);
		expect(
			press({ key: " ", repeat: true, target: button }, context).prevented,
		).toBe(true);
		expect(fake.calls).toEqual([]);
	});

	test("the first press and other targets are left alone", () => {
		const { context } = contextOf(fixture("idle"));
		expect(
			press({ key: "Enter", repeat: false, target: button }, context).prevented,
		).toBe(false);
		expect(
			press({ key: "Enter", repeat: true, target: target("INPUT") }, context)
				.prevented,
		).toBe(false);
	});
});

describe("handleRootKeyDown · stop", () => {
	test("⌘. stops the run on the stage", () => {
		const state = fixture("running");
		const id = liveStageRun(state)?.id ?? "no live run";
		const { context, fake } = contextOf(state);
		const { prevented } = press({ key: ".", mods: cmd }, context);
		expect(prevented).toBe(true);
		expect(id).not.toBe("no live run");
		expect(fake.argsOf("stop")).toEqual([[id]]);
	});

	test("⌘. takes a queued run out of the queue", () => {
		const state = fixture("queued");
		const id = liveStageRun(state)?.id ?? "no live run";
		const { context, fake } = contextOf(state);
		press({ key: ".", mods: cmd }, context);
		expect(id).not.toBe("no live run");
		expect(fake.argsOf("stop")).toEqual([[id]]);
	});

	test("an ended run on the stage has nothing to stop and the key stays the browser's", () => {
		const { context, fake } = contextOf(fixture("done"));
		const { prevented } = press({ key: ".", mods: cmd }, context);
		expect(prevented).toBe(false);
		expect(fake.calls).toEqual([]);
	});
});

describe("handleRootKeyDown · undo", () => {
	const undoable = (): FormSessionState => {
		const state = fixture("done");
		return {
			...state,
			undo: {
				seq: 9,
				kind: "fieldReset",
				values: state.rail.values,
				nextFiles: {},
				activePresetId: null,
				deletedPreset: null,
			},
		};
	};

	test("⌘Z outside a text field undoes the last change the dock reported", () => {
		const { context, fake } = contextOf(undoable());
		const { prevented } = press(
			{ key: "z", mods: cmd, target: target("BUTTON") },
			context,
		);
		expect(prevented).toBe(true);
		expect(fake.calls).toEqual([{ name: "undo", args: [] }]);
	});

	test("in a text field ⌘Z is the field's own undo", () => {
		const { context, fake } = contextOf(undoable());
		const { prevented } = press(
			{ key: "z", mods: cmd, target: target("INPUT", { type: "text" }) },
			context,
		);
		expect(prevented).toBe(false);
		expect(fake.calls).toEqual([]);
	});

	test("with nothing to undo the key is left alone", () => {
		const { context, fake } = contextOf(fixture("done"));
		expect(press({ key: "z", mods: cmd }, context).prevented).toBe(false);
		expect(fake.calls).toEqual([]);
	});
});

describe("handleRootKeyDown · shortcuts sheet", () => {
	test("⌘/ opens it anywhere, also in a text field", () => {
		const { context, fake } = contextOf(fixture("idle"));
		press({ key: "/", mods: cmd, target: target("TEXTAREA") }, context);
		expect(fake.argsOf("openOverlay")).toEqual([[{ id: "shortcuts" }]]);
	});

	test("? opens it outside a text field and types in one", () => {
		const { context, fake } = contextOf(fixture("idle"));
		const typing = press(
			{ key: "?", target: target("INPUT", { type: "text" }) },
			context,
		);
		expect(typing.prevented).toBe(false);
		expect(fake.calls).toEqual([]);
		press({ key: "?", target: target("BUTTON") }, context);
		expect(fake.argsOf("openOverlay")).toEqual([[{ id: "shortcuts" }]]);
	});

	test("with the sheet open the same keys close it", () => {
		const { context, fake } = contextOf(fixture("shortcuts"));
		press({ key: "/", mods: cmd }, context);
		expect(fake.calls).toEqual([{ name: "closeOverlay", args: [] }]);
	});

	test("inside a modal only ⌘/ is a chord", () => {
		const inside = target("BUTTON", {
			closest: (selector: string) =>
				selector === "[data-fw-modal]" ? {} : null,
		});
		const { context, fake } = contextOf(fixture("shortcuts"));
		expect(
			press({ key: "Enter", mods: cmd, target: inside }, context).prevented,
		).toBe(false);
		expect(press({ key: "?", target: inside }, context).prevented).toBe(false);
		expect(fake.calls).toEqual([]);
		press({ key: "/", mods: cmd, target: inside }, context);
		expect(fake.calls).toEqual([{ name: "closeOverlay", args: [] }]);
	});
});

describe("handleRootKeyDown · presets", () => {
	test("⌘P opens the Presets menu in any form with fields, and closes it again", () => {
		const closed = contextOf(fixture("done"));
		press({ key: "p", mods: cmd }, closed.context);
		expect(closed.fake.argsOf("openOverlay")).toEqual([[{ id: "presets" }]]);

		const base = fixture("presets");
		const open = contextOf({
			...base,
			view: { ...base.view, overlay: { id: "presets" } },
		});
		const { prevented } = press({ key: "p", mods: cmd }, open.context);
		expect(prevented).toBe(true);
		expect(open.fake.calls).toEqual([{ name: "closeOverlay", args: [] }]);
	});

	test("⌘S opens the save dialog, in update mode while a preset is active", () => {
		const idle = contextOf(fixture("idle"));
		press({ key: "s", mods: cmd }, idle.context);
		expect(idle.fake.argsOf("openOverlay")).toEqual([
			[{ id: "presetSave", mode: "save", fromRunId: null }],
		]);

		const presets = fixture("presets");
		expect(presets.rail.activePresetId).not.toBeNull();
		const active = contextOf({
			...presets,
			view: { ...presets.view, overlay: null },
		});
		press({ key: "s", mods: cmd }, active.context);
		expect(active.fake.argsOf("openOverlay")).toEqual([
			[{ id: "presetSave", mode: "update", fromRunId: null }],
		]);
	});

	test("a form without fields has no presets", () => {
		const { context, fake } = contextOf(fixture("none"));
		expect(press({ key: "s", mods: cmd }, context).prevented).toBe(false);
		expect(press({ key: "p", mods: cmd }, context).prevented).toBe(false);
		expect(fake.calls).toEqual([]);
	});
});

interface FakeInput {
	readonly el: HTMLInputElement;
	readonly clicks: () => number;
}

function fileInput(name: string, disabled = false): FakeInput {
	let clicks = 0;
	const el = {
		disabled,
		getAttribute: (attribute: string) =>
			attribute === FILE_INPUT_ATTR ? name : null,
		click: () => {
			clicks += 1;
		},
	} as unknown as HTMLInputElement;
	return { el, clicks: () => clicks };
}

function rootWith(inputs: readonly FakeInput[]) {
	return {
		querySelectorAll: () => inputs.map((input) => input.el),
	} as unknown as HTMLElement;
}

const insideFileField = (name: string) =>
	target("BUTTON", {
		closest: (selector: string) =>
			selector === `[${FILE_FIELD_ATTR}]` ? { getAttribute: () => name } : null,
	});

describe("handleRootKeyDown · ⌘O", () => {
	const invoice = () => fileInput("invoice_file");
	const supporting = () => fileInput("supporting_documents");

	test("opens the dialog of the focused file field", () => {
		const a = invoice();
		const b = supporting();
		const { context } = contextOf(fixture("idle"), rootWith([a, b]));
		const { prevented } = press(
			{ key: "o", mods: cmd, target: insideFileField("supporting_documents") },
			context,
		);
		expect(prevented).toBe(true);
		expect([a.clicks(), b.clicks()]).toEqual([0, 1]);
	});

	test("else the first empty file field, required first", () => {
		const a = invoice();
		const b = supporting();
		const { context } = contextOf(fixture("idle"), rootWith([b, a]));
		press({ key: "o", mods: cmd, target: target("BUTTON") }, context);
		expect([a.clicks(), b.clicks()]).toEqual([1, 0]);
	});

	test("a field that already holds its file is skipped", () => {
		const state = fixture("done");
		const a = invoice();
		const b = supporting();
		const { context } = contextOf(state, rootWith([a, b]));
		const held = state.rail.values.supporting_documents;
		expect(Array.isArray(held) ? held.length : 0).toBeGreaterThan(0);
		const { prevented } = press(
			{ key: "o", mods: cmd, target: target("BUTTON") },
			context,
		);
		expect(prevented).toBe(false);
		expect([a.clicks(), b.clicks()]).toEqual([0, 0]);
	});

	test("a disabled input is never clicked", () => {
		const a = fileInput("invoice_file", true);
		const b = supporting();
		const { context } = contextOf(fixture("idle"), rootWith([a, b]));
		press({ key: "o", mods: cmd, target: target("BUTTON") }, context);
		expect([a.clicks(), b.clicks()]).toEqual([0, 1]);
	});

	test("a page that cannot send FlowPath files has no file dialog to open", () => {
		const hosted = fixture("hosted-files");
		const a = invoice();
		const b = supporting();
		const { context } = contextOf(hosted, rootWith([a, b]));
		const { prevented } = press(
			{ key: "o", mods: cmd, target: target("BUTTON") },
			context,
		);
		expect(prevented).toBe(false);
		expect([a.clicks(), b.clicks()]).toEqual([0, 0]);
	});
});

describe("handleRootKeyDown · /", () => {
	test("focuses Filter fields on a form with twelve or more fields", () => {
		const focusable = {
			focused: 0,
			selected: 0,
			getAttribute: (name: string) =>
				name === "data-fw-focus" ? "filter" : null,
			focus() {
				this.focused += 1;
			},
			select() {
				this.selected += 1;
			},
		};
		const root = {
			querySelectorAll: () => [focusable],
		} as unknown as HTMLElement;
		const { context, focused } = contextOf(fixture("large"), root);
		const { prevented } = press(
			{ key: "/", target: target("BUTTON") },
			context,
		);
		expect(prevented).toBe(true);
		expect(focusable.focused).toBe(1);
		expect(focusable.selected).toBe(1);
		expect(focused).toEqual([]);
	});

	test("with the filter not on screen it switches the rail to Inputs and focuses once it is", () => {
		const base = fixture("large");
		const state: FormSessionState = {
			...base,
			rail: { ...base.rail, tab: "runs" },
			view: { ...base.view, pane: "output" },
			layout: {
				...(base.layout as NonNullable<typeof base.layout>),
				split: false,
			},
		};
		const { context, fake, focused } = contextOf(state);
		press({ key: "/", target: target("BUTTON") }, context);
		expect(fake.argsOf("setRailTab")).toEqual([["inputs"]]);
		expect(fake.argsOf("setPane")).toEqual([["inputs"]]);
		expect(focused).toEqual(["filter"]);
	});

	test("a smaller form has no filter, and a slash typed in a field is a slash", () => {
		const small = contextOf(fixture("idle"));
		expect(
			press({ key: "/", target: target("BUTTON") }, small.context).prevented,
		).toBe(false);
		const large = contextOf(fixture("large"));
		expect(
			press(
				{ key: "/", target: target("INPUT", { type: "text" }) },
				large.context,
			).prevented,
		).toBe(false);
		expect(large.fake.calls).toEqual([]);
		expect(large.focused).toEqual([]);
	});
});
