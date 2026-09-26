import { afterAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";

// bun keeps globals and module mocks for every later file in the process, so both are
// captured first and put back in afterAll.
const globalDescriptors = [
	"document",
	"Element",
	"Event",
	"HTMLElement",
	"HTMLInputElement",
	"InputEvent",
	"MouseEvent",
	"Node",
	"navigator",
	"window",
	"IS_REACT_ACT_ENVIRONMENT",
].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
// Radix picks its layout effect when first imported, so the real module loads under a document.
Object.assign(globalThis, { document: new Window().document });
const actualDialog = { ...(await import("../ui/dialog")) };

mock.module("../ui/dialog", () => ({
	...actualDialog,
	Dialog: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
	DialogBody: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
	DialogContent: ({ children }: { children?: ReactNode }) => (
		<div data-testid="dialog">{children}</div>
	),
	DialogDescription: ({ children }: { children?: ReactNode }) => (
		<p>{children}</p>
	),
	DialogFooter: ({ children }: { children?: ReactNode }) => (
		<div>{children}</div>
	),
	DialogHeader: ({ children }: { children?: ReactNode }) => (
		<div>{children}</div>
	),
	DialogTitle: ({ children }: { children?: ReactNode }) => <h2>{children}</h2>,
}));

afterAll(() => {
	mock.restore();
	mock.module("../ui/dialog", () => actualDialog);
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

function setInputValue(input: HTMLInputElement, value: string) {
	const valueSetter = Object.getOwnPropertyDescriptor(
		HTMLInputElement.prototype,
		"value",
	)?.set;
	if (!valueSetter) throw new Error("Input value setter is unavailable");
	valueSetter.call(input, value);
	input.dispatchEvent(
		new InputEvent("input", {
			bubbles: true,
			data: value,
			inputType: "insertText",
		}),
	);
	input.dispatchEvent(new Event("change", { bubbles: true }));
}

describe("WidgetSchemaListEditor", () => {
	test("adds, edits, and removes typed object items transactionally", async () => {
		const window = new Window();
		Object.assign(window, { SyntaxError, TypeError });
		Object.assign(globalThis, {
			document: window.document,
			Element: window.Element,
			Event: window.Event,
			HTMLElement: window.HTMLElement,
			HTMLInputElement: window.HTMLInputElement,
			InputEvent: window.InputEvent,
			MouseEvent: window.MouseEvent,
			Node: window.Node,
			navigator: window.navigator,
			window,
			IS_REACT_ACT_ENVIRONMENT: true,
		});
		const { createRoot } = await import("react-dom/client");
		const { WidgetSchemaListEditor } = await import("./WidgetSchemaListEditor");
		const container = document.createElement("div");
		document.body.append(container);
		const root = createRoot(container);
		const changes: unknown[][] = [];
		const schema = {
			type: "array",
			items: {
				type: "object",
				properties: {
					label: { type: "string" },
					value: { type: "number" },
				},
				required: ["label", "value"],
			},
		};
		const renderEditor = (value: unknown[]) => (
			<WidgetSchemaListEditor
				fieldName="rows"
				id="rows"
				labelledBy="rows-label"
				schema={schema}
				value={value}
				onChange={(next) => changes.push(next)}
			/>
		);

		await act(async () => {
			root.render(renderEditor([{ label: "Jan", value: 10 }]));
		});

		expect(container.textContent).toContain("1 item");
		expect(container.textContent).toContain("Jan");
		const addOverview = [...container.querySelectorAll("button")].find(
			(button) => button.textContent?.trim() === "Add item",
		);
		if (!addOverview) throw new Error("Add item button was not rendered");
		await act(async () => addOverview.click());

		const label = container.querySelector<HTMLInputElement>("#rows-item-label");
		const amount =
			container.querySelector<HTMLInputElement>("#rows-item-value");
		if (!label || !amount)
			throw new Error("Typed item fields were not rendered");
		await act(async () => {
			setInputValue(label, "Feb");
			setInputValue(amount, "25");
		});

		const addDialog = [...container.querySelectorAll("button")].find(
			(button) =>
				button !== addOverview && button.textContent?.trim() === "Add item",
		);
		if (!addDialog) throw new Error("Dialog Add item button was not rendered");
		await act(async () => addDialog.click());

		expect(changes).toEqual([
			[
				{ label: "Jan", value: 10 },
				{ label: "Feb", value: 25 },
			],
		]);

		changes.length = 0;
		await act(async () => {
			root.render(
				renderEditor([
					{ label: "Jan", value: 10 },
					{ label: "Feb", value: 25 },
				]),
			);
		});
		const editSecond = container.querySelector<HTMLButtonElement>(
			'button[aria-label="Edit rows item 2"]',
		);
		if (!editSecond) throw new Error("Edit button was not rendered");
		await act(async () => editSecond.click());
		const editedLabel =
			container.querySelector<HTMLInputElement>("#rows-item-label");
		if (!editedLabel) throw new Error("Edit fields were not rendered");
		await act(async () => setInputValue(editedLabel, "March"));
		const saveChanges = [...container.querySelectorAll("button")].find(
			(button) => button.textContent?.trim() === "Save changes",
		);
		if (!saveChanges) throw new Error("Save changes button was not rendered");
		await act(async () => saveChanges.click());
		expect(changes).toEqual([
			[
				{ label: "Jan", value: 10 },
				{ label: "March", value: 25 },
			],
		]);

		changes.length = 0;
		await act(async () => {
			root.render(
				renderEditor([
					{ label: "Jan", value: 10 },
					{ label: "March", value: 25 },
				]),
			);
		});
		const removeFirst = container.querySelector<HTMLButtonElement>(
			'button[aria-label="Remove rows item 1"]',
		);
		if (!removeFirst) throw new Error("Remove button was not rendered");
		await act(async () => removeFirst.click());
		expect(changes).toEqual([[{ label: "March", value: 25 }]]);

		await act(async () => root.unmount());
		window.close();
	});
});
