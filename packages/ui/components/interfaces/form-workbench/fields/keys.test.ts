import { describe, expect, test } from "bun:test";
import type { KeyboardEvent } from "react";
import {
	hasCommand,
	isComposing,
	isPlain,
	isPlainEnter,
	isResetChord,
	resetShortcut,
} from "./keys";

interface Press {
	readonly key: string;
	readonly metaKey: boolean;
	readonly ctrlKey: boolean;
	readonly altKey: boolean;
	readonly shiftKey: boolean;
	readonly keyCode: number;
	readonly isComposing: boolean;
}

const NOTHING: Press = {
	key: "",
	metaKey: false,
	ctrlKey: false,
	altKey: false,
	shiftKey: false,
	keyCode: 0,
	isComposing: false,
};

const press = (init: Partial<Press>) => {
	const event = { ...NOTHING, ...init };
	return {
		...event,
		nativeEvent: { isComposing: event.isComposing },
	} as unknown as KeyboardEvent;
};

describe("key guards (spec §4)", () => {
	test("an open IME composition, or the 229 key code, owns the key", () => {
		expect(isComposing(press({ key: "Enter", isComposing: true }))).toBe(true);
		expect(isComposing(press({ key: "Enter", keyCode: 229 }))).toBe(true);
		expect(isComposing(press({ key: "Enter" }))).toBe(false);
	});

	test("↵ moves on only without a modifier", () => {
		expect(isPlainEnter(press({ key: "Enter" }))).toBe(true);
		for (const modifier of [
			"metaKey",
			"ctrlKey",
			"altKey",
			"shiftKey",
		] as const) {
			expect(isPlainEnter(press({ key: "Enter", [modifier]: true }))).toBe(
				false,
			);
		}
		expect(isPlain(press({ key: "a" }))).toBe(true);
	});

	test("⇧⌘⌫ on a Mac, Ctrl+Shift+Backspace elsewhere, never with Alt", () => {
		expect(
			isResetChord(
				press({ key: "Backspace", shiftKey: true, metaKey: true }),
				true,
			),
		).toBe(true);
		expect(
			isResetChord(
				press({ key: "Backspace", shiftKey: true, ctrlKey: true }),
				true,
			),
		).toBe(false);
		expect(
			isResetChord(
				press({ key: "Backspace", shiftKey: true, ctrlKey: true }),
				false,
			),
		).toBe(true);
		expect(
			isResetChord(press({ key: "Backspace", shiftKey: true }), true),
		).toBe(false);
		expect(isResetChord(press({ key: "Backspace", metaKey: true }), true)).toBe(
			false,
		);
		expect(
			isResetChord(
				press({
					key: "Backspace",
					shiftKey: true,
					metaKey: true,
					altKey: true,
				}),
				true,
			),
		).toBe(false);
		expect(hasCommand(press({ metaKey: true }), true)).toBe(true);
		expect(hasCommand(press({ metaKey: true }), false)).toBe(false);
		expect(resetShortcut(true)).toBe("Meta+Shift+Backspace");
		expect(resetShortcut(false)).toBe("Control+Shift+Backspace");
	});
});
