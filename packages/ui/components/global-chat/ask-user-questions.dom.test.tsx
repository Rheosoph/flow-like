// @vitest-environment happy-dom
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { AskUserDraft, AskUserForm } from "../../lib/ask-user";
import { AskUserQuestions } from "./ask-user-questions";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(() => {
	act(() => root.unmount());
	host.remove();
});

const form: AskUserForm = {
	batched: false,
	questions: [
		{
			id: "pick",
			question: "Which ones?",
			mode: "multiple_choice",
			choices: [{ label: "Alpha", description: "First" }, { label: "Beta" }],
		},
	],
};

function render(drafts: AskUserDraft[]) {
	const onDraftsChange = vi.fn();
	act(() =>
		root.render(
			<AskUserQuestions
				form={form}
				drafts={drafts}
				onDraftsChange={onDraftsChange}
			/>,
		),
	);
	return onDraftsChange;
}

const choice = (label: string) =>
	[...host.querySelectorAll("label")].find((row) =>
		row.textContent?.includes(label),
	) as HTMLLabelElement;

describe("AskUserQuestions choices", () => {
	test("a choice row holds one control, never a button inside a button", () => {
		render([{ text: "", selected: [0] }]);

		expect(host.querySelector("button button")).toBeNull();
		const boxes = host.querySelectorAll('[role="checkbox"]');
		expect(boxes).toHaveLength(2);
		expect(boxes[0].getAttribute("aria-checked")).toBe("true");
		expect(choice("Alpha").htmlFor).toBe(boxes[0].id);
	});

	test("clicking the row's text or its checkbox toggles the choice once", () => {
		const onDraftsChange = render([{ text: "", selected: [0] }]);

		act(() => choice("Beta").querySelector("span")?.click());
		expect(onDraftsChange).toHaveBeenCalledTimes(1);
		expect(onDraftsChange).toHaveBeenLastCalledWith([
			{ text: "", selected: [0, 1] },
		]);

		act(() =>
			(
				choice("Alpha").querySelector('[role="checkbox"]') as HTMLButtonElement
			).click(),
		);
		expect(onDraftsChange).toHaveBeenCalledTimes(2);
		expect(onDraftsChange).toHaveBeenLastCalledWith([
			{ text: "", selected: [] },
		]);
	});
});
