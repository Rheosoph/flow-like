import type { KeyboardEvent } from "react";

/*
 * The Presets menu's keys (spec S1): ↑/↓ move through the find box, the preset rows and the actions;
 * a digit applies the preset that holds it; Delete removes the focused preset; any other letter goes
 * to the find box. Everything inside the menu, never on window.
 */

const NAV = "[data-preset-find], [data-preset-item]";
const MOVES: Readonly<Record<string, number>> = { ArrowDown: 1, ArrowUp: -1 };
const EDGES: Readonly<Record<string, number>> = { Home: 0, End: -1 };
const DIGIT = /^[1-9]$/;

export interface PanelKeys {
	readonly presets: readonly { readonly id: string; readonly digit: number }[];
	/** Digits apply presets (no digits on touch screens). */
	readonly digits: boolean;
	readonly apply: (id: string) => void;
	readonly remove: (id: string) => void;
	readonly find: () => HTMLInputElement | null;
}

const composing = (event: KeyboardEvent) =>
	event.nativeEvent.isComposing || event.keyCode === 229;
const modified = (event: KeyboardEvent) =>
	event.metaKey || event.ctrlKey || event.altKey;

const navItems = (root: HTMLElement) =>
	Array.from(root.querySelectorAll<HTMLElement>(NAV));

function focusStep(root: HTMLElement, from: Element, step: number) {
	const items = navItems(root);
	const at = items.findIndex((item) => item === from);
	const next = Math.max(0, Math.min(items.length - 1, at + step));
	items[next]?.focus();
}

function focusEdge(root: HTMLElement, edge: number) {
	const items = navItems(root);
	items.at(edge)?.focus();
}

const presetIdOf = (target: HTMLElement) =>
	target.closest<HTMLElement>("[data-preset-id]")?.dataset.presetId ?? null;

/** A key the menu answers: the page and the shell behind it do not see it (a "/" would reach the filter). */
function claim(event: KeyboardEvent) {
	event.preventDefault();
	event.stopPropagation();
}

function digitKey(event: KeyboardEvent, keys: PanelKeys) {
	if (!keys.digits || !DIGIT.test(event.key)) return false;
	const preset = keys.presets.find((item) => item.digit === Number(event.key));
	if (!preset) return false;
	claim(event);
	keys.apply(preset.id);
	return true;
}

function deleteKey(event: KeyboardEvent, keys: PanelKeys) {
	if (event.key !== "Delete" && event.key !== "Backspace") return false;
	const id = presetIdOf(event.target as HTMLElement);
	if (id === null) return false;
	claim(event);
	keys.remove(id);
	return true;
}

/** A letter or other printable key while the find box exists: it goes there (Space still presses the focused row). */
function typedKey(event: KeyboardEvent, keys: PanelKeys) {
	const find = keys.find();
	if (!find || event.key.length !== 1 || event.key === " ") return;
	event.stopPropagation();
	find.focus();
}

function edgeKey(event: KeyboardEvent<HTMLElement>) {
	const edge = EDGES[event.key];
	if (edge === undefined) return false;
	claim(event);
	focusEdge(event.currentTarget, edge);
	return true;
}

export function onPanelKey(event: KeyboardEvent<HTMLElement>, keys: PanelKeys) {
	if (composing(event) || modified(event)) return;
	const target = event.target as HTMLElement;
	const move = MOVES[event.key];
	if (move !== undefined) {
		claim(event);
		focusStep(event.currentTarget, target, move);
		return;
	}
	if (target.tagName === "INPUT") return;
	if (edgeKey(event) || digitKey(event, keys) || deleteKey(event, keys)) return;
	typedKey(event, keys);
}
