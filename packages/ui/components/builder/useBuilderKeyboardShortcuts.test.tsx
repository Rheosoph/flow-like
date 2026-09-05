import { afterEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, useRef } from "react";
import { createPortal } from "react-dom";
import { type Root, createRoot } from "react-dom/client";
import { useBuilderKeyboardShortcuts } from "./useBuilderKeyboardShortcuts";

let reactRoot: Root | undefined;
let restoreGlobals: (() => void) | undefined;

afterEach(async () => {
	await act(() => reactRoot?.unmount());
	reactRoot = undefined;
	restoreGlobals?.();
	restoreGlobals = undefined;
});

async function setup({ enabled = true, platform = "MacIntel" } = {}) {
	const testWindow = new Window();
	Object.assign(testWindow, { SyntaxError, TypeError });
	Object.defineProperty(testWindow.navigator, "platform", { value: platform });
	const globals = {
		window: testWindow,
		document: testWindow.document,
		navigator: testWindow.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const previous = Object.getOwnPropertyDescriptors(globalThis);
	Object.assign(globalThis, globals);
	restoreGlobals = () => {
		for (const key of Object.keys(globals)) {
			const descriptor = previous[key];
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};

	const actions = {
		selection: { componentIds: ["text"] },
		copy: mock(() => {}),
		cut: mock(() => {}),
		paste: mock(() => {}),
		duplicate: mock(() => {}),
		deleteComponents: mock((_ids: string[]) => {}),
		undo: mock(() => {}),
		redo: mock(() => {}),
	};
	let isEnabled = enabled;
	const host = testWindow.document.createElement("div");
	testWindow.document.body.append(host);
	function Probe() {
		const rootRef = useRef<HTMLDivElement>(null);
		useBuilderKeyboardShortcuts(rootRef, isEnabled, actions);
		return (
			<>
				<div ref={rootRef} data-builder-root="editor" tabIndex={-1}>
					<div data-canvas-background="true">
						<span data-builder-component="text">Text element</span>
						<input data-builder-component="input" />
					</div>
					<div data-hierarchy-row="text">Text row</div>
					<input data-inspector-input="true" />
					<textarea />
					<div contentEditable suppressContentEditableWarning>
						<span>Editable text</span>
					</div>
					<button type="button">Copy</button>
				</div>
				<button type="button" data-outside="true">
					Other screen action
				</button>
				<div data-builder-root="another-editor" tabIndex={-1} />
				{createPortal(
					<>
						<div data-builder-toolbar="true" data-builder-owner="editor">
							<button type="button" data-portal-button="true">
								Canvas action
							</button>
						</div>
						<div data-builder-chrome="true" data-builder-owner="editor">
							<button type="button" data-empty-container-hint="true">
								Select empty container
							</button>
						</div>
					</>,
					document.body,
				)}
			</>
		);
	}
	reactRoot = createRoot(host as unknown as HTMLElement);
	await act(() => reactRoot?.render(<Probe />));

	const find = (selector: string) => {
		const element = document.querySelector<HTMLElement>(selector);
		if (!element) throw new Error(`Missing ${selector}`);
		return element;
	};
	const key = async (
		target: HTMLElement,
		value: string,
		options: { modified?: boolean; shiftKey?: boolean; repeat?: boolean } = {},
	) => {
		const modified = options.modified ?? true;
		const event = new testWindow.KeyboardEvent("keydown", {
			key: value,
			ctrlKey: modified && !platform.includes("Mac"),
			metaKey: modified && platform.includes("Mac"),
			shiftKey: options.shiftKey,
			repeat: options.repeat,
			bubbles: true,
			cancelable: true,
		});
		await act(() => target.dispatchEvent(event as unknown as KeyboardEvent));
		return event;
	};
	return {
		actions,
		find,
		key,
		setEnabled: async (value: boolean) => {
			isEnabled = value;
			await act(() => reactRoot?.render(<Probe />));
		},
	};
}

describe("builder keyboard integration", () => {
	test.each(["MacIntel", "Win32"])(
		"binds clipboard, duplication, and history on %s",
		async (platform) => {
			const { actions, find, key } = await setup({ platform });
			const editor = find('[data-builder-root="editor"]');
			for (const value of ["c", "x", "v", "d", "z", "y"]) {
				expect((await key(editor, value)).defaultPrevented).toBe(true);
			}
			await key(editor, "z", { shiftKey: true });
			expect(actions.copy).toHaveBeenCalledTimes(1);
			expect(actions.cut).toHaveBeenCalledTimes(1);
			expect(actions.paste).toHaveBeenCalledTimes(1);
			expect(actions.duplicate).toHaveBeenCalledTimes(1);
			expect(actions.undo).toHaveBeenCalledTimes(1);
			expect(actions.redo).toHaveBeenCalledTimes(2);
		},
	);

	test("canvas and hierarchy clicks give subsequent shortcuts a focused editor", async () => {
		const { actions, find, key } = await setup();
		const editor = find('[data-builder-root="editor"]');
		for (const selector of [
			'[data-builder-component="text"]',
			'[data-builder-component="input"]',
			"[data-hierarchy-row]",
			"[data-canvas-background]",
		]) {
			find("[data-inspector-input]").focus();
			await act(() => find(selector).click());
			expect(document.activeElement).toBe(editor);
			await key(document.activeElement as HTMLElement, "c");
		}
		expect(actions.copy).toHaveBeenCalledTimes(4);
	});

	test("leaves inspector text and JSON-style editable controls to native shortcuts", async () => {
		const { actions, find, key } = await setup();
		for (const selector of [
			"[data-inspector-input]",
			"textarea",
			"[contenteditable] span",
		]) {
			const target = find(selector);
			for (const value of ["c", "x", "v", "z"]) {
				expect((await key(target, value)).defaultPrevented).toBe(false);
			}
		}
		const input = find("[data-inspector-input]");
		input.focus();
		await act(() => input.click());
		expect(document.activeElement).toBe(input);
		expect(actions.copy).not.toHaveBeenCalled();
		expect(actions.cut).not.toHaveBeenCalled();
		expect(actions.paste).not.toHaveBeenCalled();
		expect(actions.undo).not.toHaveBeenCalled();
	});

	test("handles its portaled toolbar while leaving unrelated UI and other editors alone", async () => {
		const { actions, find, key } = await setup();
		expect(
			(await key(find("[data-portal-button]"), "c")).defaultPrevented,
		).toBe(true);
		for (const target of [
			find("[data-outside]"),
			find('[data-builder-root="another-editor"]'),
			document.body,
		]) {
			expect((await key(target, "c")).defaultPrevented).toBe(false);
			expect(
				(await key(target, "Backspace", { modified: false })).defaultPrevented,
			).toBe(false);
		}
		expect(actions.copy).toHaveBeenCalledTimes(1);
		expect(actions.deleteComponents).not.toHaveBeenCalled();
	});

	test("pastes after selecting an empty container through its portaled hint", async () => {
		const { actions, find, key } = await setup();
		const hint = find("[data-empty-container-hint]");
		hint.focus();
		await act(() => hint.click());
		expect(document.activeElement).toBe(hint);
		expect(
			(await key(document.activeElement as HTMLElement, "v")).defaultPrevented,
		).toBe(true);
		expect(actions.paste).toHaveBeenCalledTimes(1);
		find("[data-builder-chrome]").setAttribute(
			"data-builder-owner",
			"another-editor",
		);
		expect((await key(hint, "v")).defaultPrevented).toBe(false);
		expect(actions.paste).toHaveBeenCalledTimes(1);
	});

	test("disables bindings in preview or dev mode and restores them on returning to edit", async () => {
		const { actions, find, key, setEnabled } = await setup();
		const editor = find('[data-builder-root="editor"]');
		await setEnabled(false);
		expect((await key(editor, "v")).defaultPrevented).toBe(false);
		expect(
			(await key(editor, "Backspace", { modified: false })).defaultPrevented,
		).toBe(false);
		expect(actions.paste).not.toHaveBeenCalled();
		await setEnabled(true);
		expect((await key(editor, "v")).defaultPrevented).toBe(true);
		expect(actions.paste).toHaveBeenCalledTimes(1);
	});

	test("keeps the root delete guard and ignores repeated destructive shortcuts", async () => {
		const { actions, find, key } = await setup();
		const editor = find('[data-builder-root="editor"]');
		actions.selection.componentIds = ["root", "text"];
		await key(editor, "Delete", { modified: false });
		expect(actions.deleteComponents).toHaveBeenCalledWith(["text"]);
		await key(editor, "Delete", { modified: false, repeat: true });
		await key(editor, "v", { repeat: true });
		expect(actions.deleteComponents).toHaveBeenCalledTimes(1);
		expect(actions.paste).not.toHaveBeenCalled();
		actions.selection.componentIds = ["root"];
		await key(editor, "Backspace", { modified: false });
		expect(actions.deleteComponents).toHaveBeenCalledTimes(1);
	});
});
