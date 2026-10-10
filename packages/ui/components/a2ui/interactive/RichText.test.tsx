import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { type ComponentProps, act } from "react";
import type { Root } from "react-dom/client";
import type { EditorUploadConfig } from "../../editor/upload-context";
import type { A2UIClientMessage, RichTextComponent } from "../types";

const browser = new Window({ url: "https://app.flow-like.test/articles" });
Object.assign(browser, { SyntaxError, TypeError, Error });
const globals = {
	window: browser,
	document: browser.document,
	navigator: browser.navigator,
	HTMLElement: browser.HTMLElement,
	Element: browser.Element,
	Node: browser.Node,
	Event: browser.Event,
	FocusEvent: browser.FocusEvent,
	IS_REACT_ACT_ENVIRONMENT: true,
};
const previousGlobals = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);

const actual = {
	actions: { ...(await import("../ActionHandler")) },
	textEditor: { ...(await import("../../ui/text-editor")) },
};
const { createRoot } = await import("react-dom/client");
const { DataProvider } = await import("../DataContext");
const { useEditorUpload } = await import("../../editor/upload-context");
const { applyElementUpdate } = await import("../apply-a2ui-message");
const events: { name: string; context: Record<string, unknown> }[] = [];
const rawActions: A2UIClientMessage[] = [];
const triggerEvent = async (
	name: string,
	_component: unknown,
	context: Record<string, unknown>,
) => {
	events.push({ name, context });
};
const onAction = (message: A2UIClientMessage) => rawActions.push(message);
type EditorProps = ComponentProps<typeof actual.textEditor.TextEditor>;
let editorProps: EditorProps;
let uploadConfig: EditorUploadConfig;

mock.module("../ActionHandler", () => ({
	...actual.actions,
	useActionContext: () => ({ appId: "app" }),
	useOnAction: () => onAction,
	useComponentEventTrigger: () => triggerEvent,
}));
mock.module("../../ui/text-editor", () => ({
	...actual.textEditor,
	TextEditor: (props: EditorProps) => {
		editorProps = props;
		uploadConfig = useEditorUpload();
		return (
			<div {...props.editorProps}>
				<button type="button">First editor control</button>
				<button type="button">Second editor control</button>
			</div>
		);
	},
}));
const { A2UIRichText, isBlankDocument } = await import("./RichText");

let root: Root;
let host: HTMLElement;
const emptyData: { path: string; value: unknown }[] = [];
const component = (
	props: Partial<RichTextComponent> = {},
): RichTextComponent => ({
	id: "body",
	type: "richText",
	value: { literalString: "" },
	debounceMs: { literalNumber: 100 },
	...props,
});
const article = (text: string) =>
	`plate_json::${JSON.stringify([{ type: "p", children: [{ text }] }])}`;

async function render(value: RichTextComponent, data = emptyData) {
	await act(async () => {
		root.render(
			<DataProvider initialData={data}>
				<A2UIRichText
					component={value}
					componentId="body"
					surfaceId="articles"
					renderChild={() => null}
				/>
			</DataProvider>,
		);
	});
}

async function change(value: string) {
	await act(async () => editorProps.onChange?.(value));
}

async function blur(inside = false) {
	const [first, second] = host.getElementsByTagName("button");
	await act(async () => {
		first.dispatchEvent(
			new browser.FocusEvent("focusout", {
				bubbles: true,
				relatedTarget: inside ? (second as never) : null,
			}) as unknown as Event,
		);
	});
}

async function settle() {
	await act(async () => new Promise((resolve) => setTimeout(resolve, 130)));
}

beforeEach(() => {
	events.length = 0;
	rawActions.length = 0;
	host = browser.document.createElement("div") as unknown as HTMLElement;
	browser.document.body.appendChild(host as never);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

afterAll(async () => {
	mock.restore();
	mock.module("../ActionHandler", () => actual.actions);
	mock.module("../../ui/text-editor", () => actual.textEditor);
	await browser.happyDOM.abort();
	for (const [key, descriptor] of previousGlobals) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

test("typed content survives read-only and disabled transitions for both bindings", async () => {
	for (const value of [{ literalString: "" }, { path: "body" }]) {
		const props = component({ value });
		const data = [{ path: "body", value: "" }];
		await render(props, data);
		await change(article("New article"));
		for (const mode of ["readOnly", "disabled"] as const) {
			await render({ ...props, [mode]: { literalBool: true } }, data);
			expect(editorProps.editable).toBe(false);
			expect(editorProps.initialContent).toBe(article("New article"));
			await render(props, data);
			expect(editorProps.initialContent).toBe(article("New article"));
		}
	}
});

test("workflow resets and authoritative revisions replace local literal edits", async () => {
	const props = component();
	await render(props);
	await change(article("Discard this draft"));
	const cleared = applyElementUpdate(
		{ id: "body", component: props },
		{ type: "setValue", value: "" },
	).component as RichTextComponent;
	await render(cleared);
	expect(editorProps.initialContent).toBe("");
	await change(article("Another draft"));
	await render({ ...cleared, documentRevision: { literalNumber: 2 } });
	expect(editorProps.initialContent).toBe("");
	await blur();
	expect(events).toEqual([
		{ name: "blur", context: { value: "", documentRevision: 2 } },
	]);
});

test("external replacement cancels the previous document's pending change", async () => {
	await render(component({ value: { literalString: article("Initial") } }));
	await change(article("Draft A"));
	await render(
		component({ value: { literalString: article("Replacement B") } }),
	);
	await settle();
	expect(editorProps.initialContent).toBe(article("Replacement B"));
	expect(events).toEqual([]);
});

test("switching article identity resets equal-valued bindings and pending events", async () => {
	await render(component({ documentId: { literalString: "a" } }));
	await change(article("Article A"));
	const abandonedChange = editorProps.onChange;
	const abandonedUpload = uploadConfig;
	await render(component({ documentId: { literalString: "b" } }));
	expect(editorProps.initialContent).toBe("");
	const rawActionCount = rawActions.length;
	await act(async () => abandonedChange?.(article("Late article A upload")));
	abandonedUpload.onUploadError?.("old.jpg", "Late failure");
	expect(rawActions).toHaveLength(rawActionCount);
	await settle();
	expect(events).toEqual([]);
	await change(article("Article B"));
	await settle();
	expect(events).toEqual([
		{
			name: "change",
			context: { documentId: "b", value: article("Article B") },
		},
	]);
});

test("placeholders follow edits and path-bound labels describe the actual editor", async () => {
	await render(
		component({
			label: { path: "label" },
			helperText: { path: "help" },
			placeholder: { path: "placeholder" },
			error: { literalBool: true },
		}),
		[
			{ path: "label", value: "Article body" },
			{ path: "help", value: "Check the sources" },
			{ path: "placeholder", value: "Start writing" },
		],
	);
	expect(host.textContent).toContain("Start writing");
	expect(host.textContent).toContain("Article body");
	expect(host.textContent).toContain("Check the sources");
	expect(editorProps.editorProps?.["aria-labelledby"]).toBe(
		"articles-body-editor-label",
	);
	expect(editorProps.editorProps?.["aria-describedby"]).toBe(
		"articles-body-editor-helper",
	);
	expect(editorProps.editorProps?.["aria-invalid"]).toBe(true);
	expect(host.getElementsByTagName("label")[0].htmlFor).toBe(
		editorProps.editorProps?.id ?? "",
	);
	await change(article("Text"));
	expect(host.textContent).not.toContain("Start writing");
	await change(article(""));
	expect(host.textContent).toContain("Start writing");
});

test("focus within the editor does not commit; leaving flushes once", async () => {
	await render(component());
	await change(article("Draft"));
	await blur(true);
	expect(events).toEqual([]);
	await blur();
	expect(events.map((event) => event.name)).toEqual(["change", "blur"]);
	await settle();
	expect(events.map((event) => event.name)).toEqual(["change", "blur"]);
	expect(rawActions.at(-1)?.context?.value).toBe(article("Draft"));
});

test("upload events include the article identity and revision", async () => {
	await render(
		component({
			documentId: { literalString: "story" },
			documentRevision: { literalNumber: 4 },
		}),
	);
	uploadConfig.onUploaded?.({
		path: "images/photo.jpg",
		url: "storage://images/photo.jpg",
		name: "photo.jpg",
		size: 40,
		type: "image/jpeg",
	});
	uploadConfig.onUploadError?.("failed.jpg", "Upload failed");
	expect(events[0]).toEqual({
		name: "imageUploaded",
		context: {
			documentId: "story",
			documentRevision: 4,
			path: "images/photo.jpg",
			url: "storage://images/photo.jpg",
			name: "photo.jpg",
			size: 40,
			type: "image/jpeg",
		},
	});
	expect(events[1]).toEqual({
		name: "imageUploadError",
		context: {
			documentId: "story",
			documentRevision: 4,
			name: "failed.jpg",
			message: "Upload failed",
		},
	});
});

test("blank detection counts media as content", () => {
	expect(isBlankDocument(article(""))).toBe(true);
	expect(isBlankDocument(article("Text"))).toBe(false);
	expect(
		isBlankDocument(
			'plate_json::[{"type":"img","url":"storage://photo.jpg","children":[{"text":""}]}]',
		),
	).toBe(false);
	expect(
		isBlankDocument(
			'plate_json::{"version":1,"children":[{"type":"p","children":[{"text":""}]}],"discussions":[],"users":{}}',
		),
	).toBe(true);
	expect(
		isBlankDocument(
			'plate_json::{"version":1,"children":[{"type":"p","children":[{"text":"Article"}]}],"discussions":[],"users":{}}',
		),
	).toBe(false);
	expect(
		isBlankDocument(
			'plate_json::[{"type":"equation","texExpression":"x=1","children":[{"text":""}]}]',
		),
	).toBe(false);
});
