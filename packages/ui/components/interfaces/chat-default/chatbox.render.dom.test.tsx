// @vitest-environment happy-dom
import { i18n } from "@flow-like/locales";
import { act, createRef } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { ChatBox, type ChatBoxModel, type ChatBoxRef } from "./chatbox";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	await i18n.changeLanguage("en");
});

/** `<ChatModelPicker {...model} />` reads `value` once per toolbar render. */
function countingModel() {
	const counter = { renders: 0 };
	const model: ChatBoxModel = {
		get value() {
			counter.renders++;
			return "a";
		},
		options: [{ id: "a", label: "Model A" }],
		onChange: () => {},
	};
	return { counter, model };
}

async function render(model: ChatBoxModel, drafts: string[] = []) {
	const ref = createRef<ChatBoxRef>();
	await act(async () => {
		root.render(
			<ChatBox
				ref={ref}
				onSendMessage={async () => {}}
				onContentChange={(draft) => drafts.push(draft)}
				fileUpload
				audioInput={false}
				model={model}
				availableTools={["Reason"]}
				defaultActiveTools={[]}
			/>,
		);
	});
	return ref;
}

async function type(value: string) {
	const textarea = host.querySelector("textarea") as HTMLTextAreaElement;
	const setValue = Object.getOwnPropertyDescriptor(
		HTMLTextAreaElement.prototype,
		"value",
	)?.set;
	await act(async () => {
		setValue?.call(textarea, value);
		textarea.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

function button(label: string) {
	return Array.from(host.querySelectorAll("button")).find(
		(candidate) => candidate.getAttribute("aria-label") === label,
	);
}

async function click(label: string) {
	await act(async () => button(label)?.click());
}

describe("ChatBox render isolation", () => {
	test("typing updates the draft without re-rendering the toolbar", async () => {
		const { counter, model } = countingModel();
		const drafts: string[] = [];
		await render(model, drafts);
		const initial = counter.renders;
		expect(initial).toBeGreaterThan(0);

		for (const draft of ["h", "he", "hel", "hello"]) await type(draft);

		expect(counter.renders).toBe(initial);
		expect(drafts.at(-1)).toBe("hello");
		expect(host.querySelector("textarea")?.value).toBe("hello");
		expect(button("Send message")?.disabled).toBe(false);
	});

	test("attachments still add, remove and clear through the memoized strip", async () => {
		const { counter, model } = countingModel();
		const ref = await render(model);
		const before = counter.renders;

		await act(async () =>
			ref.current?.addFiles?.([
				new File(["a"], "a.txt", { type: "text/plain" }),
				new File(["b"], "b.txt", { type: "text/plain" }),
			]),
		);
		expect(counter.renders).toBeGreaterThan(before);
		expect(host.textContent).toContain("a.txt");
		expect(host.textContent).toContain("b.txt");
		expect(button("Send message")?.disabled).toBe(false);

		await click("Remove a.txt");
		expect(host.textContent).not.toContain("a.txt");
		expect(host.textContent).toContain("b.txt");

		await act(async () =>
			ref.current?.addFile?.(new File(["c"], "c.txt", { type: "text/plain" })),
		);
		const clearAll = Array.from(host.querySelectorAll("button")).find(
			(candidate) => candidate.textContent === i18n.t("clearAll"),
		);
		await act(async () => clearAll?.click());
		expect(host.textContent).not.toContain("b.txt");
		expect(host.textContent).not.toContain("c.txt");
		expect(ref.current?.getAttachedFiles()).toEqual([]);
	});

	test("the memoized toolbar follows a language switch", async () => {
		const { model } = countingModel();
		await render(model);
		expect(button("Add attachment")).toBeDefined();

		await act(async () => {
			await i18n.changeLanguage("de");
		});

		expect(button("Add attachment")).toBeUndefined();
		expect(button("Anhang hinzufügen")).toBeDefined();
	});
});
