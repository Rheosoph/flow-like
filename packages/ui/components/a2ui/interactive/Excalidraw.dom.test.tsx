import type { ExcalidrawProps } from "@excalidraw/excalidraw/types";
import { validateSchema } from "@flow-like/widget-sdk";
import { act, useEffect, useRef } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DataProvider } from "../DataContext";
import { applyElementUpdate } from "../apply-a2ui-message";
import { getExcalidrawEventContextSchema } from "../excalidraw-events";
import type { ExcalidrawComponent } from "../types";
import { A2UIExcalidraw } from "./Excalidraw";

const mocks = vi.hoisted(() => ({
	editor: undefined as ExcalidrawProps | undefined,
	mounts: 0,
	onAction: vi.fn(),
	setElementValue: vi.fn(),
	trigger: vi.fn(),
	preview: true,
}));
vi.mock("../ActionHandler", () => ({
	useActionContext: () => ({ isPreviewMode: mocks.preview }),
	useComponentEventTrigger: () => mocks.trigger,
	useOnAction: () => mocks.onAction,
	useSetElementValue: () => mocks.setElementValue,
}));
vi.mock("@excalidraw/excalidraw", () => ({
	Excalidraw: (props: ExcalidrawProps) => <EditorStub {...props} />,
	restore: (data: { elements: { type: string; text?: unknown }[] }) => {
		if (
			data.elements.some(
				(element) =>
					element.type === "text" && typeof element.text !== "string",
			)
		)
			throw new Error("Invalid text element");
		return data;
	},
	serializeAsJSON: (elements: unknown, appState: unknown, files: unknown) =>
		JSON.stringify({
			type: "excalidraw",
			version: 2,
			source: "excalidraw",
			elements,
			appState,
			files,
		}),
}));

function EditorStub(props: ExcalidrawProps) {
	mocks.editor = props;
	const initialProps = useRef(props);
	useEffect(() => {
		mocks.mounts++;
		const initial = initialProps.current.initialData as {
			elements: unknown[];
			appState: object;
			files: object;
		};
		initialProps.current.onChange?.(
			initial.elements as never,
			initial.appState as never,
			initial.files as never,
		);
	}, []);
	return <div data-editor="excalidraw" />;
}

const scene = (id = "rectangle") => ({
	type: "excalidraw",
	version: 2,
	source: "test",
	elements: [{ id, type: "rectangle", version: 1 }],
	appState: { viewBackgroundColor: "#ffffff" },
	files: {},
});
const route = { name: "workflow_event", context: { nodeId: "save-drawing" } };
const component = (
	props: Partial<ExcalidrawComponent> = {},
): ExcalidrawComponent => ({
	id: "drawing",
	type: "excalidraw",
	value: { literalJson: JSON.stringify(scene()) },
	eventHandlers: { save: [route], autoSave: [route] },
	...props,
});
let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.useFakeTimers();
	vi.clearAllMocks();
	mocks.mounts = 0;
	mocks.preview = true;
	mocks.editor = undefined;
	mocks.trigger.mockImplementation(async (name, comp, _context, options) => {
		if (options.requireSuccess && !comp.eventHandlers?.[name]?.length)
			throw new Error("No save workflow configured.");
	});
	host = document.createElement("div");
	document.body.appendChild(host);
	root = createRoot(host);
});
afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	vi.useRealTimers();
});

async function render(
	comp: ExcalidrawComponent,
	data: { path: string; value: unknown }[] = [],
) {
	await act(async () => {
		root.render(
			<DataProvider initialData={data}>
				<A2UIExcalidraw
					component={comp}
					componentId="drawing"
					surfaceId="page"
					renderChild={() => null}
				/>
			</DataProvider>,
		);
	});
}
async function change(next = scene("changed")) {
	await act(async () =>
		mocks.editor?.onChange?.(
			next.elements as never,
			next.appState as never,
			next.files as never,
		),
	);
}
async function save() {
	await act(async () => host.querySelector("button")?.click());
}
const events = (name: string) =>
	mocks.trigger.mock.calls.filter(([event]) => event === name);

describe("Excalidraw A2UI renderer", () => {
	it("loads JSON once and sends schema-valid typed saves including embedded files", async () => {
		await render(
			component({ value: { literalString: JSON.stringify(scene()) } }),
		);
		expect(events("load")).toHaveLength(1);
		await act(async () => vi.advanceTimersByTimeAsync(10000));
		expect(events("autoSave")).toHaveLength(0);
		const changed = {
			...scene("image"),
			files: {
				image: {
					id: "image",
					mimeType: "image/png",
					dataURL: "data:image/png;base64,AAAA",
					created: 1,
				},
			},
		};
		await change(changed);
		await save();
		const [, , context, options] = events("save")[0];
		expect(context.scene.files).toEqual(changed.files);
		expect(context.changeCount).toBe(1);
		expect(options).toEqual({
			legacyFallback: false,
			wildcardFallback: false,
			requireSuccess: true,
		});
		for (const [name, , payload] of mocks.trigger.mock.calls) {
			expect(
				validateSchema(getExcalidrawEventContextSchema(name), payload).valid,
			).toBe(true);
		}
		expect(host.textContent).toContain("No unsaved changes");
	});

	it("keeps literal and path edits in the editor and replaces them on workflow value writes", async () => {
		const props = component({ value: { path: "drawing" } });
		const data = [{ path: "drawing", value: scene() }];
		await render(props, data);
		await change();
		await render({ ...props, height: { literalString: "700px" } }, data);
		expect(mocks.mounts).toBe(1);
		const updated = applyElementUpdate(
			{ id: "drawing", component: props },
			{ type: "setValue", value: scene("replacement") },
		);
		await render(updated.component as ExcalidrawComponent, data);
		expect(mocks.mounts).toBe(2);
		expect(events("load").at(-1)?.[2].scene.elements[0].id).toBe("replacement");
		expect(mocks.setElementValue.mock.calls.at(-1)?.[1].elements[0].id).toBe(
			"replacement",
		);
		await act(async () => vi.advanceTimersByTimeAsync(5000));
		expect(events("autoSave")).toHaveLength(0);
	});

	it("an identical workflow value write resets unsaved literal edits", async () => {
		const props = component();
		await render(props);
		await change();
		const updated = applyElementUpdate(
			{ id: "drawing", component: props },
			{ type: "setValue", value: scene() },
		);
		await render(updated.component as ExcalidrawComponent);
		expect(mocks.mounts).toBe(2);
		expect(host.textContent).toContain("No unsaved changes");
	});

	it("debounces content edits, ignores viewport changes, and intercepts Ctrl+S", async () => {
		await render(component({ autoSaveDelayMs: { literalNumber: 200 } }));
		await change();
		await act(async () => vi.advanceTimersByTimeAsync(150));
		await change({
			...scene("changed"),
			appState: { ...scene().appState, scrollX: 10 },
		} as ReturnType<typeof scene>);
		await act(async () => vi.advanceTimersByTimeAsync(50));
		expect(events("autoSave")).toHaveLength(1);
		await change(scene("another"));
		const key = new KeyboardEvent("keydown", {
			key: "s",
			ctrlKey: true,
			bubbles: true,
			cancelable: true,
		});
		await act(async () =>
			host.querySelector("[data-editor]")?.dispatchEvent(key),
		);
		expect(key.defaultPrevented).toBe(true);
		expect(events("save")).toHaveLength(1);
		await act(async () => vi.advanceTimersByTimeAsync(1000));
		expect(events("autoSave")).toHaveLength(1);
	});

	it("reports invalid input and recovers when a valid scene arrives", async () => {
		await render(component({ value: { literalString: "invalid json" } }));
		expect(host.querySelector("[role=alert]")).not.toBeNull();
		expect(events("error")[0][2].operation).toBe("load");
		expect(mocks.mounts).toBe(0);
		await render(component());
		expect(events("load")).toHaveLength(1);
		expect(host.querySelector("[role=alert]")).toBeNull();
	});

	it("reports upstream restore failures before mounting a broken editor", async () => {
		await render(
			component({
				value: {
					literalJson: JSON.stringify({
						...scene(),
						elements: [
							{
								id: "bad",
								type: "text",
								text: { bad: true },
								width: 150,
								height: 30,
							},
						],
					}),
				},
			}),
		);
		expect(mocks.mounts).toBe(0);
		expect(events("error").at(-1)?.[2]).toEqual({
			actionId: "error",
			operation: "load",
			message: "Invalid text element",
		});
		await render(component());
		expect(mocks.mounts).toBe(1);
		expect(host.querySelector("[role=alert]")).toBeNull();
	});

	it("requires exact handlers and keeps edits dirty when saving fails", async () => {
		await render(
			component({ eventHandlers: { "*": [route] }, actions: [route] }),
		);
		await change();
		await act(async () => vi.advanceTimersByTimeAsync(10000));
		expect(events("autoSave")).toHaveLength(0);
		await save();
		expect(host.textContent).toContain("Unsaved changes");
		expect(host.querySelector("[role=alert]")?.textContent).toContain(
			"No save workflow",
		);
		expect(events("error").at(-1)?.[2].operation).toBe("save");
	});

	it("cancels pending autosave when read-only and avoids actions in the builder", async () => {
		const props = component();
		await render(props);
		await change();
		await render({ ...props, readOnly: { literalBool: true } });
		await act(async () => vi.advanceTimersByTimeAsync(5000));
		expect(events("autoSave")).toHaveLength(0);
		expect(host.querySelector("button")).toBeNull();
		expect(mocks.editor?.viewModeEnabled).toBe(true);
		mocks.preview = false;
		mocks.trigger.mockClear();
		await render(
			component({ value: { literalJson: JSON.stringify(scene("builder")) } }),
		);
		expect(mocks.trigger).not.toHaveBeenCalled();
	});
});
