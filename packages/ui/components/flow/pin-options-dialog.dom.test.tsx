import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, useState } from "react";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	Event: window.Event,
	Node: window.Node,
	IS_REACT_ACT_ENVIRONMENT: true,
};
// bun keeps globals and module mocks for every later file in the process, so both are
// captured first and put back in afterAll.
const globalDescriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);
const actualLocales = { ...(await import("@flow-like/locales")) };
const translate = (
	_key: string,
	fallback: string,
	variables: Record<string, unknown> = {},
) =>
	fallback.replace(/\{\{(\w+)\}\}/g, (_, key) => String(variables[key] ?? ""));
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate }),
}));

const { createRoot } = await import("react-dom/client");
const { ValueChipsInput } = await import("./pin-options-dialog");

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});
afterAll(async () => {
	mock.restore();
	mock.module("@flow-like/locales", () => actualLocales);
	await window.happyDOM.close();
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

async function setup(initial: string[] = []) {
	const state = { values: initial, pending: "" };
	function Harness() {
		const [value, setValue] = useState(state);
		state.values = value.values;
		state.pending = value.pending;
		return (
			<ValueChipsInput
				values={value.values}
				pending={value.pending}
				onChange={setValue}
			/>
		);
	}
	const host = window.document.createElement("div");
	window.document.body.append(host);
	const root = createRoot(host as unknown as HTMLElement);
	await act(async () => root.render(<Harness />));
	cleanup = async () => {
		await act(async () => root.unmount());
		host.remove();
	};
	const input = () => {
		const element = host.querySelector("input");
		if (!element) throw new Error("Chip input is missing");
		return element;
	};
	const setValue = Object.getOwnPropertyDescriptor(
		window.HTMLInputElement.prototype,
		"value",
	)?.set;
	return {
		state,
		input,
		chips: () =>
			Array.from(host.querySelectorAll("[data-slot='badge']")).map(
				(chip) => chip.textContent,
			),
		type: async (text: string) => {
			for (const char of text) {
				await act(async () => {
					setValue?.call(input(), input().value + char);
					input().dispatchEvent(new window.Event("input", { bubbles: true }));
				});
			}
		},
		press: async (key: string) => {
			await act(async () => {
				input().dispatchEvent(
					new window.KeyboardEvent("keydown", { key, bubbles: true }),
				);
			});
		},
		click: async (label: string) => {
			const button = host.querySelector(`button[aria-label="${label}"]`);
			if (!button) throw new Error(`Button "${label}" is missing`);
			await act(async () => (button as unknown as HTMLButtonElement).click());
		},
	};
}

test("typing a comma at the end turns the word into a chip", async () => {
	const chips = await setup();
	await chips.type("low,");
	expect(chips.state.values).toEqual(["low"]);
	expect(chips.input().value).toBe("");
	await chips.type(" medium, high");
	expect(chips.state.values).toEqual(["low", "medium"]);
	expect(chips.input().value).toBe("high");
});

test("Enter adds the pending value and Backspace on an empty input removes the last chip", async () => {
	const chips = await setup(["a"]);
	await chips.type("b");
	await chips.press("Enter");
	expect(chips.state.values).toEqual(["a", "b"]);
	await chips.press("Backspace");
	expect(chips.state.values).toEqual(["a"]);
});

test("a chip can be removed with its button", async () => {
	const chips = await setup(["low", "high"]);
	expect(chips.chips()).toEqual(["low", "high"]);
	await chips.click("Remove low");
	expect(chips.state.values).toEqual(["high"]);
});
