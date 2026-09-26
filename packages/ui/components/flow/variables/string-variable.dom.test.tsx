import { afterAll, afterEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import {
	IValueType,
	type IVariable,
	IVariableType,
} from "../../../lib/schema/flow/variable";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../lib/uint8";
import { StringVariable } from "./string-variable";

const window = new Window();
Object.assign(window, { SyntaxError, TypeError });
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	HTMLTextAreaElement: window.HTMLTextAreaElement,
	Event: window.Event,
	Node: window.Node,
	IS_REACT_ACT_ENVIRONMENT: true,
};
const descriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);
const { createRoot } = await import("react-dom/client");

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});
afterAll(async () => {
	await window.happyDOM.close();
	for (const [key, descriptor] of descriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

async function setup(value = "", disabled = false) {
	const host = window.document.createElement("div");
	window.document.body.append(host);
	const container = host as unknown as HTMLDivElement;
	const root = createRoot(container);
	let current: IVariable = {
		id: "certificate",
		name: "Certificate",
		data_type: IVariableType.String,
		value_type: IValueType.Normal,
		exposed: false,
		editable: true,
		secret: true,
		default_value: convertJsonToUint8Array(value),
	};
	const updates: IVariable[] = [];
	const render = () =>
		root.render(
			<StringVariable
				variable={current}
				disabled={disabled}
				onChange={(next) => {
					current = next;
					updates.push(next);
					render();
				}}
			/>,
		);
	await act(async () => render());
	cleanup = async () => {
		await act(async () => root.unmount());
		container.remove();
	};
	return {
		container,
		updates,
		value: () => parseUint8ArrayToJson(current.default_value),
		input: () => {
			const input = container.querySelector<HTMLInputElement>("input");
			if (!input) throw new Error("Secret input is missing");
			return input;
		},
	};
}

async function paste(input: HTMLInputElement, value: string) {
	const clipboardData = new window.DataTransfer();
	clipboardData.setData("text/plain", value);
	const event = new window.ClipboardEvent("paste", {
		bubbles: true,
		cancelable: true,
		clipboardData,
	});
	await act(async () => input.dispatchEvent(event as unknown as Event));
	return event;
}

async function click(container: HTMLElement, label: string) {
	const button = container.querySelector<HTMLButtonElement>(
		`button[aria-label="${label}"]`,
	);
	expect(button).not.toBeNull();
	await act(async () => button?.click());
}

test("pasting a long certificate preserves every byte while concealed", async () => {
	const editor = await setup();
	const certificate = `-----BEGIN CERTIFICATE-----\r\n${"A".repeat(64)}\r\n${"QUJD".repeat(16).concat("\r\n").repeat(4096)}-----END CERTIFICATE-----\r\n`;
	const event = await paste(editor.input(), certificate);
	expect(event.defaultPrevented).toBe(true);
	expect(editor.value()).toBe(certificate);
	expect(editor.updates).toHaveLength(1);
	expect(editor.input().type).toBe("password");
	expect(editor.input().readOnly).toBe(true);
	expect(editor.container.querySelector("textarea")).toBeNull();
	expect(editor.container.textContent).not.toContain("BEGIN CERTIFICATE");

	await click(editor.container, "Show secret value");
	expect(
		editor.container.querySelector("textarea")?.value.replaceAll("\r\n", "\n"),
	).toBe(certificate.replaceAll("\r\n", "\n"));
	await click(editor.container, "Hide secret value");
	expect(editor.value()).toBe(certificate);
	expect(editor.updates).toHaveLength(1);
});

test("a saved multiline secret can be replaced while concealed and cleared", async () => {
	const editor = await setup("old\ncertificate\n");
	expect(editor.updates).toHaveLength(0);
	await paste(editor.input(), "replacement\ncertificate\n");
	expect(editor.value()).toBe("replacement\ncertificate\n");
	await paste(editor.input(), "single-line replacement");
	expect(editor.value()).toBe("single-line replacement");
	expect(editor.input().readOnly).toBe(false);
	await click(editor.container, "Clear value");
	expect(editor.value()).toBe("");
});

test("multiline paste replaces the selected part of a single-line value", async () => {
	const editor = await setup("prefix replace suffix");
	editor.input().setSelectionRange(7, 14);
	await paste(editor.input(), "line one\nline two");
	expect(editor.value()).toBe("prefix line one\nline two suffix");
});

test("long single-line secrets remain editable without truncation", async () => {
	const editor = await setup("initial");
	const value = "secret".repeat(50_000);
	const setValue = Object.getOwnPropertyDescriptor(
		window.HTMLInputElement.prototype,
		"value",
	)?.set;
	await act(async () => {
		setValue?.call(editor.input(), value);
		editor.input().dispatchEvent(new Event("input", { bubbles: true }));
	});
	expect(editor.value()).toBe(value);
	expect(editor.input().value).toBe(value);
});

test("revealed multiline secrets remain editable and can be hidden again", async () => {
	const editor = await setup("first\nsecond\n");
	await click(editor.container, "Show secret value");
	const textarea = editor.container.querySelector("textarea");
	if (!textarea) throw new Error("Revealed secret editor is missing");
	const setValue = Object.getOwnPropertyDescriptor(
		window.HTMLTextAreaElement.prototype,
		"value",
	)?.set;
	await act(async () => {
		setValue?.call(textarea, "first\nupdated\nthird\n");
		textarea.dispatchEvent(new Event("input", { bubbles: true }));
	});
	await click(editor.container, "Hide secret value");
	expect(editor.value()).toBe("first\nupdated\nthird\n");
	expect(editor.input().type).toBe("password");
	expect(editor.container.querySelector("textarea")).toBeNull();
});

test("disabled secret editors reject paste and reveal actions", async () => {
	const editor = await setup("original\nsecret", true);
	await paste(editor.input(), "replacement\nsecret");
	await click(editor.container, "Show secret value");
	expect(editor.updates).toHaveLength(0);
	expect(editor.container.querySelector("textarea")).toBeNull();
});
