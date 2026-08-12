import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { DEFAULT_SHORTCUTS, createShortcutManager } from "./KeyboardShortcuts";

const DELETE_SHORTCUTS = DEFAULT_SHORTCUTS.filter(
	(shortcut) => shortcut.action === "delete",
);

function createTestWindow() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		HTMLElement: window.HTMLElement,
		KeyboardEvent: window.KeyboardEvent,
		navigator: window.navigator,
		window,
	});
	return window;
}

describe("builder delete shortcuts", () => {
	test.each(["Backspace", "Delete"])(
		"handles %s and prevents its browser default",
		(key) => {
			const window = createTestWindow();
			const actions: string[] = [];
			const manager = createShortcutManager(
				(action) => actions.push(action),
				DELETE_SHORTCUTS,
			);
			manager.bind();

			const event = new window.KeyboardEvent("keydown", {
				key,
				bubbles: true,
				cancelable: true,
			});
			window.document.body.dispatchEvent(event);
			manager.unbind();

			expect(actions).toEqual(["delete"]);
			expect(event.defaultPrevented).toBe(true);
		},
	);

	test.each(["input", "textarea", "select"])(
		"does not intercept Backspace inside %s",
		(tagName) => {
			const window = createTestWindow();
			const actions: string[] = [];
			const manager = createShortcutManager(
				(action) => actions.push(action),
				DELETE_SHORTCUTS,
			);
			manager.bind();

			const target = window.document.createElement(tagName);
			window.document.body.append(target);
			const event = new window.KeyboardEvent("keydown", {
				key: "Backspace",
				bubbles: true,
				cancelable: true,
			});
			target.dispatchEvent(event);
			manager.unbind();

			expect(actions).toEqual([]);
			expect(event.defaultPrevented).toBe(false);
		},
	);

	test("does not intercept Backspace inside a content-editable element", () => {
		const window = createTestWindow();
		const actions: string[] = [];
		const manager = createShortcutManager(
			(action) => actions.push(action),
			DELETE_SHORTCUTS,
		);
		manager.bind();

		const editor = window.document.createElement("div");
		editor.contentEditable = "true";
		const target = window.document.createElement("span");
		editor.append(target);
		window.document.body.append(editor);
		const event = new window.KeyboardEvent("keydown", {
			key: "Backspace",
			bubbles: true,
			cancelable: true,
		});
		target.dispatchEvent(event);
		manager.unbind();

		expect(actions).toEqual([]);
		expect(event.defaultPrevented).toBe(false);
	});

	test("prevents Backspace even when there is nothing to delete", () => {
		const window = createTestWindow();
		const manager = createShortcutManager(() => {}, DELETE_SHORTCUTS);
		manager.bind();

		const event = new window.KeyboardEvent("keydown", {
			key: "Backspace",
			bubbles: true,
			cancelable: true,
		});
		window.document.body.dispatchEvent(event);
		manager.unbind();

		expect(event.defaultPrevented).toBe(true);
	});

	test("respects a nested control that already handled the key", () => {
		const window = createTestWindow();
		const actions: string[] = [];
		const manager = createShortcutManager(
			(action) => actions.push(action),
			DELETE_SHORTCUTS,
		);
		manager.bind();

		const target = window.document.createElement("button");
		target.addEventListener("keydown", (event) => event.preventDefault());
		window.document.body.append(target);
		const event = new window.KeyboardEvent("keydown", {
			key: "Backspace",
			bubbles: true,
			cancelable: true,
		});
		target.dispatchEvent(event);
		manager.unbind();

		expect(actions).toEqual([]);
		expect(event.defaultPrevented).toBe(true);
	});
});
