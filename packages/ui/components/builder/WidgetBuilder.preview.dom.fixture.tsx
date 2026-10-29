import { afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { IWidgetRef } from "../../state/backend-state/page-state";
import type {
	A2UIServerMessage,
	Surface,
	SurfaceComponent,
} from "../a2ui/types";
import type { ActionContext, BuilderContextType } from "./BuilderContext";

const window = new Window({ url: "https://builder.local" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	localStorage: window.localStorage,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: (callback: FrameRequestCallback) =>
		setTimeout(() => callback(0), 0),
	cancelAnimationFrame: (id: number) => clearTimeout(id),
	IS_REACT_ACT_ENVIRONMENT: true,
});
const noop = () => {};
const Children = ({ children }: { children: ReactNode }) => children;
const backend = { boardState: {} };
const dndKit = await import("@dnd-kit/core");
const backendState = await import("../../state/backend-state");
const locales = await import("@flow-like/locales");
const a2uiRenderer = await import("../a2ui/A2UIRenderer");
let builder: BuilderContextType | undefined;
let togglePreview: (() => void) | undefined;
let sendMessage: ((message: A2UIServerMessage) => void) | undefined;

mock.module("@flow-like/locales", () => ({
	...locales,
	useTranslation: () => ({
		t: (key: string, fallback?: string) => fallback ?? key,
	}),
}));
mock.module("../../hooks/use-asset-source", () => ({
	useAssetSource: () => ({ src: undefined }),
}));
mock.module("../../state/backend-state", () => ({
	...backendState,
	useBackend: () => backend,
}));
mock.module("../a2ui/A2UIRenderer", () => ({
	...a2uiRenderer,
	A2UIRenderer: ({
		surface,
		onA2UIMessage,
	}: {
		surface: Surface;
		onA2UIMessage: (message: A2UIServerMessage) => void;
	}) => {
		sendMessage = onA2UIMessage;
		return <output>{JSON.stringify(surface.components)}</output>;
	},
}));
mock.module("../scoped-custom-css", () => ({ ScopedCustomCss: () => null }));
mock.module("../ui/resizable", () => ({
	ResizablePanel: Children,
	ResizablePanelGroup: Children,
	ResizableHandle: () => null,
}));
mock.module("../ui/tabs", () => ({
	Tabs: Children,
	TabsContent: Children,
	TabsList: Children,
	TabsTrigger: Children,
}));
mock.module("./BuilderDndContext", () => ({
	BuilderDndProvider: Children,
	useBuilderDnd: () => ({ activeId: null }),
}));
mock.module("@dnd-kit/core", () => ({
	...dndKit,
	useDroppable: () => ({ setNodeRef: noop, isOver: false }),
}));
mock.module("./BuilderDragOverlay", () => ({ BuilderDragOverlay: () => null }));
mock.module("./BuilderRenderer", () => ({ BuilderRenderer: () => null }));
mock.module("./ComponentPalette", () => ({ ComponentPalette: () => null }));
mock.module("./DevModePanel", () => ({ DevModePanel: () => null }));
mock.module("./HierarchyTree", () => ({ HierarchyTree: () => null }));
mock.module("./Inspector", () => ({ Inspector: () => null }));
mock.module("./ResponsivePreview", () => ({ ResponsivePreview: Children }));
mock.module("./a2ui-copilot", () => ({ A2UICopilot: () => null }));
mock.module("./useBuilderKeyboardShortcuts", () => ({
	useBuilderKeyboardShortcuts: noop,
}));
mock.module("./Toolbar", () => ({
	Toolbar: ({ onPreview }: { onPreview: () => void }) => {
		builder = useBuilder();
		togglePreview = onPreview;
		return null;
	},
}));

const { useBuilder } = await import("./BuilderContext");
const { WidgetBuilder } = await import("./WidgetBuilder");
let root: Root | undefined;

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	window.document.body.innerHTML = "";
	builder = undefined;
	togglePreview = undefined;
	sendMessage = undefined;
});

const title = (text: string): SurfaceComponent => ({
	id: "title",
	component: { id: "title", type: "text", content: { literalString: text } },
});
const initialComponents: SurfaceComponent[] = [
	{
		id: "root",
		component: {
			id: "root",
			type: "column",
			children: { explicitList: ["title"] },
		},
	},
	title("Original title"),
];

function currentBuilder() {
	if (!builder) throw new Error("Builder did not mount");
	return builder;
}

async function mount() {
	const host = window.document.createElement("div");
	window.document.body.append(host);
	root = createRoot(host as unknown as HTMLElement);
	const render = async (context: ActionContext = {}) => {
		await act(() =>
			root?.render(
				<WidgetBuilder
					initialComponents={initialComponents}
					actionContext={{ appId: "app", pageId: "page", ...context }}
					externalAssistant
				/>,
			),
		);
	};
	await render();
	await act(() => togglePreview?.());
	return { host, render };
}

async function showRuntimeTitle(text: string) {
	if (!sendMessage) throw new Error("Preview did not mount");
	await act(() =>
		sendMessage?.({
			type: "surfaceUpdate",
			surfaceId: "page",
			components: [title(text)],
		}),
	);
}

test("raw JSON replaces a runtime preview without a reload and remains editable", async () => {
	const { host } = await mount();
	expect(host.textContent).toContain("Original title");
	await showRuntimeTitle("Workflow title");
	expect(host.textContent).toContain("Workflow title");
	await act(() => {
		const json = JSON.parse(currentBuilder().getRawJson());
		json.components = [initialComponents[0], title("Applied JSON title")];
		expect(currentBuilder().setRawJson(JSON.stringify(json))).toBe(true);
	});
	expect(host.textContent).toContain("Applied JSON title");
	expect(host.textContent).not.toContain("Workflow title");
	await showRuntimeTitle("New workflow title");
	expect(host.textContent).toContain("New workflow title");
	await act(() => currentBuilder().undo());
	expect(host.textContent).toContain("Original title");
});

test("selection and board metadata retain runtime state while widget changes reset it", async () => {
	const { host, render } = await mount();
	await showRuntimeTitle("Workflow title");
	await act(() => currentBuilder().selectComponent("title"));
	expect(host.textContent).toContain("Workflow title");
	await render({ workflowEvents: [{ nodeId: "event", name: "Refreshed" }] });
	expect(host.textContent).toContain("Workflow title");
	await act(() =>
		currentBuilder().addWidgetRef("instance", {
			id: "widget",
			rootComponentId: "root",
			components: initialComponents,
		} as IWidgetRef),
	);
	expect(host.textContent).toContain("Original title");
});
