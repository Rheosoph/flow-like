import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import {
	FILE_FIELD_ATTR,
	FILE_INPUT_ATTR,
	FOCUS_VALUE,
	FORM_LIMITS,
	type FormSessionActions,
	type FormSessionState,
	LIVE_RUN_STATUSES,
} from "../contracts";
import { blockedNames } from "../model/validate";
import { isEmpty, isFileField } from "../model/values";
import { stageRunOf } from "../run/run-view";
import { findFocusable, selectContents } from "./focus";

/** What the root's `onKeyDown` knows how to do; the keys themselves are the spec's §4 map. */
export type Chord =
	| "run"
	| "runLeave"
	| "stop"
	| "chooseFiles"
	| "presets"
	| "savePreset"
	| "undo"
	| "shortcuts"
	| "help"
	| "filter";

export interface KeyFacts {
	readonly key: string;
	readonly metaKey: boolean;
	readonly ctrlKey: boolean;
	readonly altKey: boolean;
	readonly shiftKey: boolean;
}

const MOD_KEY_CHORD: Readonly<Record<string, Chord>> = {
	".": "stop",
	o: "chooseFiles",
	p: "presets",
	s: "savePreset",
	z: "undo",
	"/": "shortcuts",
};

const PLAIN_KEY_CHORD: Readonly<Record<string, Chord>> = {
	"?": "help",
	"/": "filter",
};

const keyOf = (facts: KeyFacts) =>
	facts.key.length === 1 ? facts.key.toLowerCase() : facts.key;

/** `mod`: ⌘ on macOS or Ctrl elsewhere, with the other one up. `foreign`: any other ⌘ or Ctrl. */
function modifierOf(facts: KeyFacts, mac: boolean) {
	const own = mac ? facts.metaKey : facts.ctrlKey;
	const other = mac ? facts.ctrlKey : facts.metaKey;
	if (own && !other) return "mod";
	return own || other ? "foreign" : "none";
}

/** Alt never takes part; Shift belongs to ⇧⌘↵ and to the layouts that type "/" with it. */
function modChord(facts: KeyFacts): Chord | null {
	const key = keyOf(facts);
	if (key === "Enter") return facts.shiftKey ? "runLeave" : "run";
	if (facts.shiftKey) return key === "/" ? "shortcuts" : null;
	return MOD_KEY_CHORD[key] ?? null;
}

/**
 * The chord a key press is, wherever focus is. Shift is part of a layout's "/" and "?", so those two
 * ignore it; a modified key that is no chord is the browser's.
 */
export function chordOf(facts: KeyFacts, mac: boolean): Chord | null {
	const modifier = modifierOf(facts, mac);
	if (facts.altKey || modifier === "foreign") return null;
	return modifier === "mod"
		? modChord(facts)
		: (PLAIN_KEY_CHORD[keyOf(facts)] ?? null);
}

/** Chords that make sense while typing; the others are text there. */
const ALLOWED_IN_TEXT: ReadonlySet<Chord> = new Set<Chord>([
	"run",
	"runLeave",
	"stop",
	"chooseFiles",
	"presets",
	"savePreset",
	"shortcuts",
]);

const TEXT_INPUT_TYPES: ReadonlySet<string> = new Set([
	"",
	"text",
	"search",
	"email",
	"url",
	"tel",
	"password",
	"number",
	"date",
	"datetime-local",
	"month",
	"time",
	"week",
]);

const TEXT_ROLES: ReadonlySet<string> = new Set([
	"textbox",
	"searchbox",
	"spinbutton",
]);

const EDITABLE_ATTRIBUTES: ReadonlySet<string> = new Set([
	"",
	"true",
	"plaintext-only",
]);

const isEditable = (element: HTMLElement) =>
	element.isContentEditable === true ||
	EDITABLE_ATTRIBUTES.has(element.getAttribute?.("contenteditable") ?? "-") ||
	TEXT_ROLES.has(element.getAttribute?.("role") ?? "");

/** Focus is in something the person types into: text inputs, text areas, selects, editable content. */
export function isTextEntry(target: EventTarget | null | undefined) {
	const element = target as HTMLElement | null | undefined;
	if (typeof element?.tagName !== "string") return false;
	const tag = element.tagName.toLowerCase();
	if (tag === "input")
		return TEXT_INPUT_TYPES.has(
			((element as HTMLInputElement).type ?? "").toLowerCase(),
		);
	return tag === "textarea" || tag === "select" || isEditable(element);
}

/** An IME is composing: the Enter that confirms a word never moves on or runs. */
export function isComposingKey(
	event: Pick<ReactKeyboardEvent, "keyCode" | "nativeEvent">,
) {
	return event.nativeEvent?.isComposing === true || event.keyCode === 229;
}

function closestElement(target: EventTarget | null, selector: string) {
	const element = target as Element | null;
	return typeof element?.closest === "function"
		? element.closest(selector)
		: null;
}

/** The interface root's modal frames (leave, shortcuts, save preset) take every key but ⌘/. */
const inModal = (target: EventTarget | null) =>
	closestElement(target, "[data-fw-modal]") !== null;

/** A held Enter or Space clicks a focused button again and again. */
function suppressHeldButton(event: ReactKeyboardEvent<HTMLElement>) {
	const pressesButton = event.key === "Enter" || event.key === " ";
	if (
		event.repeat &&
		pressesButton &&
		closestElement(event.target, "button, [role='button']")
	)
		event.preventDefault();
}

/** The run on the stage when it can still be stopped or taken out of the queue. */
export function liveStageRun(state: FormSessionState) {
	const run = stageRunOf(state);
	return run && LIVE_RUN_STATUSES.includes(run.status) ? run : null;
}

export interface KeyContext {
	readonly state: FormSessionState;
	readonly actions: FormSessionActions;
	readonly root: HTMLElement | null;
	/** Focus an element by its `data-fw-focus` value once the next render has put it in the DOM. */
	readonly focusSoon: (value: string) => void;
}

interface ChordRule {
	applies(context: KeyContext, event: ReactKeyboardEvent<HTMLElement>): boolean;
	run(context: KeyContext, event: ReactKeyboardEvent<HTMLElement>): void;
}

const hasFields = ({ state }: KeyContext) => state.form.fields.length > 0;

/** The hidden file input of the focused file field, else of the first empty one (required first). */
export function fileInputFor(
	context: KeyContext,
	target: EventTarget | null,
): HTMLInputElement | null {
	const { root, state } = context;
	if (!root) return null;
	const inputs = Array.from(
		root.querySelectorAll<HTMLInputElement>(`input[${FILE_INPUT_ATTR}]`),
	).filter((input) => !input.disabled);
	const inputNamed = (name: string) =>
		inputs.find((input) => input.getAttribute(FILE_INPUT_ATTR) === name) ??
		null;
	const focusedName = closestElement(
		target,
		`[${FILE_FIELD_ATTR}]`,
	)?.getAttribute(FILE_FIELD_ATTR);
	const focused = focusedName ? inputNamed(focusedName) : null;
	if (focused) return focused;

	const { fields, host } = state.form;
	const blocked = blockedNames(fields, host);
	const empty = fields.filter(
		(field) =>
			isFileField(field) &&
			!blocked.includes(field.name) &&
			isEmpty(field, state.rail.values[field.name]),
	);
	const ordered = [
		...empty.filter((field) => field.required),
		...empty.filter((field) => !field.required),
	];
	for (const field of ordered) {
		const input = inputNamed(field.name);
		if (input) return input;
	}
	return null;
}

const toggleSheet: ChordRule = {
	applies: () => true,
	run: ({ state, actions }) => {
		if (state.view.overlay?.id === "shortcuts") actions.closeOverlay();
		else actions.openOverlay({ id: "shortcuts" });
	},
};

const CHORDS: Readonly<Record<Chord, ChordRule>> = {
	run: {
		applies: () => true,
		run: ({ actions }) => actions.run({ leaveAsIs: false, from: "chord" }),
	},
	runLeave: {
		applies: hasFields,
		run: ({ actions }) => actions.run({ leaveAsIs: true, from: "chord" }),
	},
	stop: {
		applies: ({ state }) => liveStageRun(state) !== null,
		run: ({ state, actions }) => {
			const run = liveStageRun(state);
			if (run) actions.stop(run.id);
		},
	},
	chooseFiles: {
		applies: (context, event) => fileInputFor(context, event.target) !== null,
		run: (context, event) => fileInputFor(context, event.target)?.click(),
	},
	presets: {
		applies: hasFields,
		run: ({ state, actions }) => {
			if (state.view.overlay?.id === "presets") actions.closeOverlay();
			else actions.openOverlay({ id: "presets" });
		},
	},
	savePreset: {
		applies: hasFields,
		run: ({ state, actions }) =>
			actions.openOverlay({
				id: "presetSave",
				mode: state.rail.activePresetId === null ? "save" : "update",
				fromRunId: null,
			}),
	},
	undo: {
		applies: ({ state }) => state.undo !== null,
		run: ({ actions }) => actions.undo(),
	},
	shortcuts: toggleSheet,
	help: toggleSheet,
	filter: {
		applies: ({ state }) =>
			state.form.fields.length >= FORM_LIMITS.filterFromFields,
		run: ({ root, state, actions, focusSoon }) => {
			const filter = root ? findFocusable(root, FOCUS_VALUE.filter) : null;
			if (filter) {
				filter.focus();
				selectContents(filter);
				return;
			}
			if (state.rail.tab !== "inputs") actions.setRailTab("inputs");
			if (state.layout?.split === false && state.view.pane !== "inputs")
				actions.setPane("inputs");
			focusSoon(FOCUS_VALUE.filter);
		},
	},
};

/** A modal frame takes every key but ⌘/; typing keys are text while focus is in a text control. */
function chordReaches(chord: Chord, target: EventTarget | null) {
	if (inModal(target)) return chord === "shortcuts";
	return ALLOWED_IN_TEXT.has(chord) || !isTextEntry(target);
}

/**
 * The interface root's keyboard (spec §4): chords that work anywhere inside the form, with Ctrl for
 * ⌘ outside macOS. A key that runs ignores a held key; every key is ignored while an IME composes;
 * a control that already handled the key keeps it.
 */
export function handleRootKeyDown(
	event: ReactKeyboardEvent<HTMLElement>,
	context: KeyContext,
) {
	if (event.defaultPrevented || isComposingKey(event)) return;
	suppressHeldButton(event);
	const chord = chordOf(event, context.state.form.viewer.mac);
	if (!chord || !chordReaches(chord, event.target)) return;
	const rule = CHORDS[chord];
	if (!rule.applies(context, event)) return;
	event.preventDefault();
	if (!event.repeat) rule.run(context, event);
}
