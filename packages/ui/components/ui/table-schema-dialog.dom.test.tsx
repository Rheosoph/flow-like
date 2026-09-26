import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { IIndexConfig } from "../../state/backend-state/db-state";
import type { LanceSchema } from "./lance-viewer";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	HTMLButtonElement: window.HTMLButtonElement,
	Element: window.Element,
	Node: window.Node,
	Text: window.Text,
	DocumentFragment: window.DocumentFragment,
	NodeFilter: window.NodeFilter,
	MutationObserver: window.MutationObserver,
	ResizeObserver: window.ResizeObserver,
	Event: window.Event,
	CustomEvent: window.CustomEvent,
	InputEvent: window.InputEvent,
	KeyboardEvent: window.KeyboardEvent,
	FocusEvent: window.FocusEvent,
	MouseEvent: window.MouseEvent,
	PointerEvent: window.PointerEvent,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: (callback: FrameRequestCallback) =>
		setTimeout(() => callback(0), 0),
	cancelAnimationFrame: (id: number) => clearTimeout(id),
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
	fallback: string | Record<string, unknown> = "",
	variables: Record<string, unknown> = {},
) => {
	const options = typeof fallback === "string" ? variables : fallback;
	const text =
		typeof fallback === "string"
			? fallback
			: String(
					options.count === 1
						? options.defaultValue_one
						: (options.defaultValue_other ?? options.defaultValue ?? ""),
				);
	return text.replace(/\{\{(\w+)\}\}/g, (_, key) => String(options[key] ?? ""));
};
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate }),
}));

const { createRoot } = await import("react-dom/client");
const { TableSchemaDialog } = await import("./table-schema-dialog");

afterAll(() => {
	mock.restore();
	mock.module("@flow-like/locales", () => actualLocales);
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
	for (const cleanup of cleanups.splice(0)) await cleanup();
});

const schema: LanceSchema = {
	table: "entities",
	primaryKey: "id",
	fields: [
		{ name: "id", kind: "string", nullable: false, keyEligible: true },
		{ name: "source_id", kind: "string", nullable: false, keyEligible: true },
		{ name: "label", kind: "string", nullable: true },
	],
};

const indices: IIndexConfig[] = [
	{ name: "id_idx", index_type: "BTree", columns: ["id"] },
];

type Props = Parameters<typeof TableSchemaDialog>[0];

async function openDialog(props: Partial<Props> = {}) {
	const container = window.document.createElement("div");
	window.document.body.appendChild(container);
	const root = createRoot(container as unknown as HTMLElement);
	cleanups.push(async () => {
		await act(async () => root.unmount());
		container.remove();
	});
	await act(async () =>
		root.render(
			<TableSchemaDialog
				schema={schema}
				tableName="entities"
				rowCount={12480}
				onGetIndices={async () => indices}
				{...props}
			/>,
		),
	);
	await act(async () => buttonNamed("Schema", container).click());
	const dialog = window.document.querySelector('[role="dialog"]');
	if (!dialog) throw new Error("schema dialog did not open");
	return dialog;
}

/** happy-dom nodes don't satisfy lib.dom's `ParentNode`, so scopes are typed structurally. */
type Scope = { querySelectorAll(selectors: string): ArrayLike<unknown> };

function buttons(scope: Scope = window.document.body) {
	return Array.from(scope.querySelectorAll("button")) as HTMLButtonElement[];
}

function buttonNamed(label: string, scope?: Scope) {
	const match = buttons(scope).find(
		(button) =>
			button.textContent?.trim() === label ||
			button.getAttribute("aria-label") === label,
	);
	if (!match) throw new Error(`no button named "${label}"`);
	return match;
}

function railItem(name: string) {
	const match = buttons().find(
		(button) =>
			button.querySelector(".font-mono")?.textContent === name &&
			button.closest("nav"),
	);
	if (!match) throw new Error(`no rail item for "${name}"`);
	return match;
}

async function click(button: HTMLButtonElement) {
	await act(async () => button.click());
}

async function type(input: HTMLInputElement, value: string) {
	const setter = Object.getOwnPropertyDescriptor(
		window.HTMLInputElement.prototype,
		"value",
	)?.set;
	await act(async () => {
		setter?.call(input, value);
		input.dispatchEvent(
			new window.InputEvent("input", { bubbles: true }) as unknown as Event,
		);
	});
}

test("dropping a column waits for confirmation", async () => {
	const dropped: string[][] = [];
	const dialog = await openDialog({
		onDropColumns: async (columns) => {
			dropped.push(columns);
		},
	});
	await click(railItem("label"));
	await click(buttonNamed("Drop column"));
	expect(dropped).toEqual([]);
	expect(dialog.textContent).toContain("Drop label?");
	await click(buttonNamed("Drop label"));
	expect(dropped).toEqual([["label"]]);
});

test("a failed drop keeps the confirmation open", async () => {
	const dialog = await openDialog({
		onDropColumns: async () => {
			throw new Error("Commit conflict");
		},
	});
	await click(railItem("label"));
	await click(buttonNamed("Drop column"));
	await click(buttonNamed("Drop label"));
	expect(dialog.textContent).toContain("Drop label?");
	expect(buttonNamed("Drop label").disabled).toBe(false);
});

test("the key column can't be made nullable or dropped", async () => {
	const altered: [string, boolean][] = [];
	const dialog = await openDialog({
		onAlterColumn: async (column, nullable) => {
			altered.push([column, nullable]);
		},
		onDropColumns: async () => {},
	});
	expect(dialog.querySelector("h3")?.textContent).toContain("id");
	expect(dialog.textContent).toContain("Key stays required");
	expect(() => buttonNamed("Allow NULL")).toThrow();
	expect(dialog.textContent).toContain("The table key can't be dropped");
	expect(() => buttonNamed("Drop column")).toThrow();

	await click(railItem("source_id"));
	await click(buttonNamed("Allow NULL"));
	expect(altered).toEqual([]);
	await click(buttonNamed("Allow NULL"));
	expect(altered).toEqual([["source_id", true]]);
});

test("a nullable column can be made required again", async () => {
	const altered: [string, boolean][] = [];
	await openDialog({
		onAlterColumn: async (column, nullable) => {
			altered.push([column, nullable]);
		},
	});
	await click(railItem("label"));
	await click(buttonNamed("Make required"));
	expect(altered).toEqual([["label", false]]);
});

test("Set as key is disabled for columns that can't be the key", async () => {
	const keyed: string[] = [];
	const keyless = { ...schema, primaryKey: undefined };
	await openDialog({
		schema: keyless,
		onSetPrimaryKey: async (column) => {
			keyed.push(column);
		},
	});
	await click(railItem("label"));
	expect(buttonNamed("Set as key").disabled).toBe(true);

	await click(railItem("source_id"));
	await click(buttonNamed("Set as key"));
	expect(keyed).toEqual([]);
	await click(buttonNamed("Set as key"));
	expect(keyed).toEqual(["source_id"]);
});

test("a new column needs a literal that matches its type", async () => {
	const added: [string, string][] = [];
	const dialog = await openDialog({
		onAddColumn: async (name, expression) => {
			added.push([name, expression]);
		},
	});
	await click(buttonNamed("New column"));
	const name = dialog.querySelector(
		'input[placeholder="column_name"]',
	) as unknown as HTMLInputElement;
	await type(name, "confidence");
	const double = buttons(dialog).find((button) =>
		button.textContent?.startsWith("Double"),
	);
	if (!double) throw new Error("no Double type tile");
	await click(double);
	await click(buttonNamed("A value"));
	const value = dialog.querySelector(
		'input[aria-label="Default value"]',
	) as unknown as HTMLInputElement;
	await type(value, "abc");
	await click(buttonNamed("Add Column"));
	expect(added).toEqual([]);
	expect(dialog.textContent).toContain("Enter a number");

	await type(value, "0.5");
	await click(buttonNamed("Add Column"));
	expect(added).toEqual([["confidence", "CAST(0.5 AS DOUBLE)"]]);
});

test("without write callbacks the dialog is read-only", async () => {
	const dialog = await openDialog();
	expect(dialog.textContent).toContain("Read-only");
	for (const label of [
		"New column",
		"Drop column",
		"Allow NULL",
		"Set as key",
		"Drop index",
	]) {
		expect(() => buttonNamed(label)).toThrow();
	}
});
