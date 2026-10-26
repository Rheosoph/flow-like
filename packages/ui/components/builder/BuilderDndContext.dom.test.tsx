// @vitest-environment happy-dom
import type { DndContextProps, DragEndEvent } from "@dnd-kit/core";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { IWidget } from "../../state/backend-state/widget-state";
import type { SurfaceComponent } from "../a2ui/types";
import {
	type BuilderContextType,
	BuilderProvider,
	useBuilder,
} from "./BuilderContext";
import { BuilderDndProvider, WIDGET_DND_TYPE } from "./BuilderDndContext";
import { getExplicitChildren } from "./componentTree";

const mocks = vi.hoisted(() => ({
	onDragEnd: undefined as DndContextProps["onDragEnd"],
	widgetState: {
		getWidget: vi.fn(),
		getWidgetAuthoritative: vi.fn(),
		getWidgetMeta: vi.fn(),
		getWidgets: vi.fn(),
		updateWidget: vi.fn(),
		pushWidgetMeta: vi.fn(),
		deleteWidget: vi.fn(),
	},
	getUserWidgets: vi.fn(),
	invalidate: vi.fn(),
	toastError: vi.fn(),
}));

vi.mock("@dnd-kit/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@dnd-kit/core")>()),
	DndContext: ({ children, onDragEnd }: DndContextProps) => {
		mocks.onDragEnd = onDragEnd;
		return children;
	},
}));
vi.mock("../../state/backend-state", () => ({
	useBackend: () => ({
		widgetState: mocks.widgetState,
		userState: { getUserWidgets: mocks.getUserWidgets },
	}),
}));
vi.mock("../../hooks/use-invoke", () => ({
	useInvalidateInvoke: () => mocks.invalidate,
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (_key: string, fallback: string) => fallback,
	}),
}));
vi.mock("sonner", () => ({ toast: { error: mocks.toastError } }));

const column = (id: string, children: string[] = []): SurfaceComponent => ({
	id,
	component: { id, type: "column", children: { explicitList: children } },
});
const text = (id: string): SurfaceComponent => ({
	id,
	component: { id, type: "text", content: { literalString: id } },
});
const sourceWidget = (): IWidget => ({
	id: "source-widget",
	name: "Shared card",
	rootComponentId: "card",
	components: [column("card", ["title"]), text("title")],
	dataModel: [],
	customizationOptions: [],
	exposedProps: [
		{
			id: "heading",
			label: "Heading",
			targetComponentId: "title",
			propertyPath: "content",
			propType: "String",
		},
	],
	tags: ["shared"],
	createdAt: "2026-09-01T00:00:00Z",
	updatedAt: "2026-09-01T00:00:00Z",
});

let root: Root;
let host: HTMLDivElement;
let builder: BuilderContextType | undefined;
let source: IWidget;

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.resetAllMocks();
	mocks.onDragEnd = undefined;
	builder = undefined;
	source = sourceWidget();
	mocks.widgetState.getWidget.mockResolvedValue(source);
	mocks.widgetState.getWidgetAuthoritative.mockResolvedValue(source);
	mocks.widgetState.getWidgetMeta.mockResolvedValue(undefined);
	mocks.widgetState.updateWidget.mockResolvedValue(undefined);
	mocks.widgetState.pushWidgetMeta.mockResolvedValue(undefined);
	mocks.widgetState.deleteWidget.mockResolvedValue(undefined);
	mocks.invalidate.mockResolvedValue(undefined);
	localStorage.clear();
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(() => root.unmount());
	host.remove();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

const context = () => {
	if (!builder) throw new Error("Builder did not mount");
	return builder;
};

async function mountBuilder() {
	function Reader() {
		builder = useBuilder();
		return null;
	}
	await act(() =>
		root.render(
			<BuilderProvider
				initialComponents={[column("root", ["existing"]), text("existing")]}
				actionContext={{ appId: "target-app" }}
			>
				<BuilderDndProvider setIsDraggingGlobal={() => {}}>
					<Reader />
				</BuilderDndProvider>
			</BuilderProvider>,
		),
	);
}

function drop(appId: string, index = 0) {
	if (!mocks.onDragEnd) throw new Error("Drag handler did not mount");
	return mocks.onDragEnd({
		active: {
			id: "palette-widget",
			data: {
				current: { type: WIDGET_DND_TYPE, appId, widgetId: source.id },
			},
		},
		collisions: [
			{
				id: "root-drop-zone",
				data: { dropData: { type: "drop-zone", parentId: "root", index } },
			},
		],
	} as unknown as DragEndEvent);
}

function placedWidget() {
	const componentId = getExplicitChildren(context().getComponent("root"))[0];
	const instance = context().getComponent(componentId)?.component;
	if (instance?.type !== "widgetInstance")
		throw new Error("Widget was not placed");
	return {
		componentId,
		instance,
		widget: context().getWidgetRef(instance.instanceId),
	};
}

test("a foreign widget is saved in the target project before one undoable placement", async () => {
	const original = structuredClone(source);
	let finishSave!: () => void;
	const saving = new Promise<void>((resolve) => {
		finishSave = resolve;
	});
	mocks.widgetState.updateWidget.mockImplementation(() => saving);
	await mountBuilder();
	let dropped: ReturnType<typeof drop>;
	await act(async () => {
		dropped = drop("source-app");
		await Promise.resolve();
	});
	expect(mocks.widgetState.updateWidget).toHaveBeenCalledTimes(1);
	expect(context().components.size).toBe(2);
	expect(context().widgetRefs.size).toBe(0);
	expect(context().canUndo).toBe(false);
	await act(async () => {
		finishSave();
		await dropped;
	});

	const [targetApp, copied] = mocks.widgetState.updateWidget.mock.calls[0] as [
		string,
		IWidget,
	];
	expect(targetApp).toBe("target-app");
	expect(copied.id).not.toBe(source.id);
	expect(mocks.widgetState.getWidgetAuthoritative).toHaveBeenCalledWith(
		"source-app",
		source.id,
		undefined,
	);
	expect(mocks.widgetState.pushWidgetMeta).toHaveBeenCalledWith(
		"target-app",
		copied.id,
		expect.objectContaining({ name: source.name }),
	);
	const copiedIds = new Set(copied.components.map((component) => component.id));
	expect(copiedIds.has(copied.rootComponentId)).toBe(true);
	for (const component of copied.components) {
		expect(["card", "title"]).not.toContain(component.id);
		expect(component.component.id).toBe(component.id);
		for (const child of getExplicitChildren(component))
			expect(copiedIds.has(child)).toBe(true);
	}
	expect(copiedIds.has(copied.exposedProps?.[0].targetComponentId ?? "")).toBe(
		true,
	);
	expect(source).toEqual(original);
	const { componentId, instance, widget } = placedWidget();
	expect(instance).toMatchObject({ appId: "target-app", widgetId: copied.id });
	expect(widget).toMatchObject({
		id: copied.id,
		rootComponentId: copied.rootComponentId,
		components: copied.components,
		exposedProps: copied.exposedProps,
	});
	expect(getExplicitChildren(context().getComponent("root"))).toEqual([
		componentId,
		"existing",
	]);
	expect(mocks.invalidate).toHaveBeenCalledWith(mocks.widgetState.getWidgets, [
		"target-app",
	]);
	expect(mocks.invalidate).toHaveBeenCalledWith(mocks.getUserWidgets, []);
	await act(() => context().undo());
	expect(context().components.size).toBe(2);
	expect(context().widgetRefs.size).toBe(0);
	expect(context().canUndo).toBe(false);
	await act(() => context().redo());
	expect(placedWidget().instance).toEqual(instance);
	expect(placedWidget().widget).toEqual(widget);
});

test("a pending import preserves parent edits and inserts at the requested index", async () => {
	let finishSave!: () => void;
	const saving = new Promise<void>((resolve) => {
		finishSave = resolve;
	});
	mocks.widgetState.updateWidget.mockImplementation(() => saving);
	await mountBuilder();
	let dropped: ReturnType<typeof drop>;
	await act(async () => {
		dropped = drop("source-app", 1);
		await Promise.resolve();
	});
	expect(mocks.widgetState.updateWidget).toHaveBeenCalledTimes(1);
	await act(() => {
		context().addComponent(text("added-while-saving"));
		context().updateComponent("root", {
			component: column("root", ["added-while-saving", "existing"]).component,
		});
	});
	await act(async () => {
		finishSave();
		await dropped;
	});
	const children = getExplicitChildren(context().getComponent("root"));
	expect(children).toEqual([
		"added-while-saving",
		expect.any(String),
		"existing",
	]);
	expect(context().getComponent(children[1])?.component).toMatchObject({
		type: "widgetInstance",
		appId: "target-app",
	});
	expect(context().getComponent("added-while-saving")).toEqual(
		text("added-while-saving"),
	);
	await act(() => context().undo());
	expect(getExplicitChildren(context().getComponent("root"))).toEqual([
		"added-while-saving",
		"existing",
	]);
	expect(context().widgetRefs.size).toBe(0);
});

test("imports completing together keep both instances and the existing child", async () => {
	let finishSaves!: () => void;
	const saving = new Promise<void>((resolve) => {
		finishSaves = resolve;
	});
	mocks.widgetState.updateWidget.mockImplementation(() => saving);
	await mountBuilder();
	const dropped: ReturnType<typeof drop>[] = [];
	await act(async () => {
		dropped.push(drop("source-app"), drop("source-app"));
		await Promise.resolve();
	});
	expect(mocks.widgetState.updateWidget).toHaveBeenCalledTimes(2);
	await act(async () => {
		finishSaves();
		await Promise.all(dropped);
	});
	const children = getExplicitChildren(context().getComponent("root"));
	expect(children).toHaveLength(3);
	expect(children[2]).toBe("existing");
	expect(new Set(children).size).toBe(3);
	const instances = children.slice(0, 2).map((id) => {
		const instance = context().getComponent(id)?.component;
		if (instance?.type !== "widgetInstance")
			throw new Error("Imported widget was not placed");
		expect(context().getWidgetRef(instance.instanceId)?.id).toBe(
			instance.widgetId,
		);
		return instance;
	});
	expect(instances[0].instanceId).not.toBe(instances[1].instanceId);
	expect(instances[0].widgetId).not.toBe(instances[1].widgetId);
	expect(context().widgetRefs.size).toBe(2);
});

test("a failed project copy reports the error and leaves the page unchanged", async () => {
	mocks.widgetState.updateWidget.mockRejectedValue(new Error("Write failed"));
	vi.spyOn(console, "error").mockImplementation(() => {});
	await mountBuilder();
	await act(async () => {
		await drop("source-app");
	});
	expect(mocks.toastError).toHaveBeenCalledWith(
		"Failed to add widget to this project",
	);
	expect(context().components.size).toBe(2);
	expect(getExplicitChildren(context().getComponent("root"))).toEqual([
		"existing",
	]);
	expect(context().widgetRefs.size).toBe(0);
	expect(context().canUndo).toBe(false);
	expect(mocks.invalidate).not.toHaveBeenCalled();
});

test("a widget from the current project is placed without making a copy", async () => {
	await mountBuilder();
	await act(async () => {
		await drop("target-app");
	});
	expect(mocks.widgetState.getWidget).toHaveBeenCalledWith(
		"target-app",
		source.id,
	);
	expect(mocks.widgetState.getWidgetAuthoritative).not.toHaveBeenCalled();
	expect(mocks.widgetState.updateWidget).not.toHaveBeenCalled();
	expect(mocks.widgetState.pushWidgetMeta).not.toHaveBeenCalled();
	expect(placedWidget().instance).toMatchObject({
		appId: "target-app",
		widgetId: source.id,
	});
	expect(placedWidget().widget?.components).toEqual(source.components);
});
