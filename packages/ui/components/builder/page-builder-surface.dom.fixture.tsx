import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import {
	ICommandType,
	type IGenericCommand,
} from "../../lib/schema/flow/board/commands/generic-command";
import type { INode } from "../../lib/schema/flow/node";
import type { IPage } from "../../state/backend-state/page-state";
import type { SurfaceComponent } from "../a2ui/types";
import type { BuilderContextType } from "./BuilderContext";
import type { WidgetBuilderProps } from "./WidgetBuilder";
import {
	WORKFLOW_EVENT_NODE_GAP,
	type WorkflowEventInfo,
} from "./page-workflow-events";

const window = new Window({
	url: "https://builder.local/page-builder?id=page&app=app",
});
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	localStorage: window.localStorage,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	HTMLButtonElement: window.HTMLButtonElement,
	SVGElement: window.SVGElement,
	Element: window.Element,
	Node: window.Node,
	NodeFilter: window.NodeFilter,
	DocumentFragment: window.DocumentFragment,
	DOMRect: window.DOMRect,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	CustomEvent: window.CustomEvent,
	FocusEvent: window.FocusEvent,
	KeyboardEvent: window.KeyboardEvent,
	MouseEvent: window.MouseEvent,
	PointerEvent: window.PointerEvent,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: (callback: FrameRequestCallback) =>
		setTimeout(() => callback(0), 0),
	cancelAnimationFrame: (id: number) => clearTimeout(id),
	ResizeObserver: class {
		observe() {}
		unobserve() {}
		disconnect() {}
	},
	IS_REACT_ACT_ENVIRONMENT: true,
});

type DomElement = NonNullable<ReturnType<typeof window.document.querySelector>>;
interface HandlerAction {
	name: string;
	context: Record<string, unknown>;
}
interface ComponentActions {
	eventHandlers?: Record<string, HandlerAction[]>;
	actions?: HandlerAction[];
}
interface Scenario {
	pageBoardId?: string;
	catalog: INode[];
	catalogError?: Error;
	commandError?: Error;
	holdCommands: boolean;
	boardGate?: Promise<void>;
}

const unboundWorkflowEvent = (): HandlerAction[] => [
	{ name: "workflow_event", context: {} },
];
const page: IPage = {
	id: "page",
	name: "Map page",
	boardId: "saved-board",
	layoutType: "freeform",
	content: [],
	components: [
		{
			id: "map",
			component: {
				id: "map",
				type: "microWidgetInstance",
				instanceId: "map-instance",
				packageId: "gods-eye-view",
				widgetId: "gods-eye-view",
				packageVersion: "1.0.0",
				contract: {
					contractVersion: 1,
					id: "gods-eye-view",
					events: { entityClicked: { description: "Entity selection" } },
				},
				props: {},
				actionBindings: {},
				eventHandlers: { "*": unboundWorkflowEvent() },
			},
		},
		{
			id: "run",
			component: {
				id: "run",
				type: "button",
				label: { literalString: "Run" },
				eventHandlers: { click: unboundWorkflowEvent() },
				actions: unboundWorkflowEvent(),
			} as unknown as SurfaceComponent["component"],
		},
	],
	createdAt: "2026-09-01T00:00:00Z",
	updatedAt: "2026-09-01T00:00:00Z",
};
const catalogTemplate = (name: string, friendlyName: string) =>
	({
		id: `${name}-template`,
		name,
		friendly_name: friendlyName,
		category: "Events",
		description: friendlyName,
		coordinates: [0, 0, 0],
		pins: {
			"template-exec-out": {
				id: "template-exec-out",
				name: "exec_out",
				friendly_name: "Output",
				connected_to: [],
				depends_on: [],
			},
		},
	}) as unknown as INode;
const catalog = [
	catalogTemplate("events_simple", "Simple Event"),
	catalogTemplate("events_widget_action", "Widget Action Event"),
];
const catalogSnapshot = structuredClone(catalog);
const savedPages: IPage[] = [];
const sentCommands: IGenericCommand[] = [];
const executedCommands: IGenericCommand[] = [];
const heldCommands: (() => void)[] = [];
let scenario: Scenario = { catalog, holdCommands: false };
let boardReads = 0;
let catalogReads = 0;
let builderProps: WidgetBuilderProps | undefined;
let builder: BuilderContextType | undefined;

const backend = {
	pageState: {
		getPages: async () => [{ pageId: page.id, name: page.name }],
		getPage: async () =>
			structuredClone({ ...page, boardId: scenario.pageBoardId }),
		updatePage: async (_appId: string, updated: IPage) => {
			savedPages.push(structuredClone(updated));
		},
	},
	appState: { getApp: async () => ({ frontend: {} }) },
	boardState: {
		getBoard: async (appId: string, boardId: string) => {
			expect(appId).toBe("app");
			expect(boardId).toBe("saved-board");
			boardReads++;
			await scenario.boardGate;
			return {
				nodes: {
					widget: {
						id: "widget-node",
						name: "events_widget_action",
						friendly_name: savedPages.length
							? "Updated widget schema"
							: "Widget selection",
						coordinates: [0, 0, 0],
					},
					simple: {
						id: "simple-node",
						name: "events_simple",
						friendly_name: "Refresh",
						coordinates: [300, 120, 0],
					},
					...Object.fromEntries(
						executedCommands.flatMap((command) =>
							command.node ? [[command.node.id, command.node]] : [],
						),
					),
				},
			};
		},
		getCatalog: async (appId: string) => {
			expect(appId).toBe("app");
			catalogReads++;
			if (scenario.catalogError) throw scenario.catalogError;
			return scenario.catalog;
		},
		executeCommand: async (
			appId: string,
			boardId: string,
			command: IGenericCommand,
		) => {
			expect(appId).toBe("app");
			expect(boardId).toBe("saved-board");
			if (scenario.holdCommands) {
				await new Promise<void>((resolve) => heldCommands.push(resolve));
			}
			if (scenario.commandError) throw scenario.commandError;
			sentCommands.push(structuredClone(command));
			const executed: IGenericCommand = structuredClone(command);
			if (executed.node)
				executed.node.id = `server-node-${sentCommands.length}`;
			executedCommands.push(executed);
			return structuredClone(executed);
		},
	},
};

const translate = (
	_key: string,
	fallback: string | Record<string, unknown>,
	values?: Record<string, unknown>,
) => {
	const options = typeof fallback === "object" ? fallback : (values ?? {});
	const template =
		typeof fallback === "string"
			? fallback
			: ((options.count === 1
					? options.defaultValue_one
					: (options.defaultValue_other ?? options.defaultValue)) as string);
	return (template ?? "").replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
		String(options[name] ?? ""),
	);
};
const locales = await import("@flow-like/locales");
const backendState = await import("../../state/backend-state");
const widgetBuilder = await import("./WidgetBuilder");
mock.module("@flow-like/locales", () => ({
	...locales,
	useTranslation: () => ({ t: translate }),
}));
mock.module("../../state/backend-state", () => ({
	...backendState,
	useBackend: () => backend,
}));
mock.module("./WidgetBuilder", () => ({
	...widgetBuilder,
	WidgetBuilder: (props: WidgetBuilderProps) => {
		builderProps = props;
		return (
			<BuilderProvider
				initialComponents={props.initialComponents}
				actionContext={props.actionContext}
			>
				<BuilderReader />
				<Inspector />
			</BuilderProvider>
		);
	},
}));

const { BuilderProvider, useBuilder } = await import("./BuilderContext");
const { Inspector } = await import("./Inspector");
const { PageBuilderSurface } = await import("./page-builder-surface");
let root: Root | undefined;
let client: QueryClient | undefined;

function BuilderReader() {
	builder = useBuilder();
	return null;
}

async function settle(check: () => boolean) {
	for (let attempt = 0; attempt < 100; attempt++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 5));
		});
		if (check()) return;
	}
	throw new Error("Page builder did not reach the expected state");
}

async function mount(overrides: Partial<Scenario> = {}) {
	scenario = {
		pageBoardId: "saved-board",
		catalog,
		holdCommands: false,
		...overrides,
	};
	boardReads = 0;
	catalogReads = 0;
	savedPages.length = 0;
	sentCommands.length = 0;
	executedCommands.length = 0;
	heldCommands.length = 0;
	builderProps = undefined;
	builder = undefined;
	const mountedClient = new QueryClient({
		defaultOptions: {
			queries: { retry: false, staleTime: Number.POSITIVE_INFINITY },
		},
	});
	client = mountedClient;
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);
	await act(async () =>
		root?.render(
			<QueryClientProvider client={mountedClient}>
				<PageBuilderSurface appId="app" pageId="page" />
			</QueryClientProvider>,
		),
	);
	await settle(() =>
		scenario.pageBoardId
			? builderProps?.actionContext?.workflowEvents?.length === 2
			: !!builder,
	);
}

function currentBuilder() {
	if (!builderProps) throw new Error("Page builder did not mount");
	return builderProps;
}

function liveActions(componentId: string): ComponentActions {
	const component = builder?.components.get(componentId);
	if (!component) throw new Error(`Component ${componentId} is not mounted`);
	return component.component as ComponentActions;
}

function withBinding() {
	const components = structuredClone(currentBuilder().initialComponents ?? []);
	components[0].component.eventHandlers = {
		entityClicked: [
			{
				name: "workflow_event",
				context: {
					nodeId: "widget-node",
					appId: currentBuilder().actionContext?.appId,
					boardId: currentBuilder().actionContext?.boardId,
				},
			},
		],
	};
	return components;
}

function find(selector: string, text?: string): DomElement {
	const match = [...window.document.querySelectorAll(selector)].find(
		(element) => text === undefined || element.textContent?.trim() === text,
	);
	if (!match) throw new Error(`Nothing matches ${selector} "${text ?? ""}"`);
	return match;
}

const createEventButtons = () => [
	...window.document.querySelectorAll(
		'button[aria-label="Create event in flow"]',
	),
];

async function dispatch(target: DomElement, event: Event) {
	await act(async () => {
		target.dispatchEvent(event as never);
	});
}

const click = (target: DomElement) =>
	dispatch(
		target,
		new window.MouseEvent("click", {
			bubbles: true,
			cancelable: true,
		}) as unknown as Event,
	);
const mouseDown = (target: DomElement) =>
	dispatch(
		target,
		new window.MouseEvent("mousedown", {
			bubbles: true,
			cancelable: true,
			button: 0,
		}) as unknown as Event,
	);
const pointerDown = (target: DomElement) =>
	dispatch(
		target,
		new window.PointerEvent("pointerdown", {
			bubbles: true,
			cancelable: true,
			button: 0,
			pointerType: "mouse",
		}) as unknown as Event,
	);
const press = (target: DomElement, key: string) =>
	dispatch(
		target,
		new window.KeyboardEvent("keydown", {
			key,
			bubbles: true,
			cancelable: true,
		}) as unknown as Event,
	);

async function openActionsFor(componentId: string) {
	await act(async () => builder?.setSelection({ componentIds: [componentId] }));
	await mouseDown(find('[role="tab"]', "Actions"));
}

function releaseHeldCommand() {
	const release = heldCommands.shift();
	if (!release) throw new Error("No create is waiting on the board");
	release();
}

afterEach(async () => {
	for (const release of heldCommands.splice(0)) release();
	await act(async () => root?.unmount());
	root = undefined;
	client?.clear();
	window.document.body.replaceChildren();
});
afterAll(() => {
	mock.restore();
	window.happyDOM.cancelAsync();
});

test("loads the saved board and saves a page widget handler without Instantiate Widget", async () => {
	await mount();
	expect(currentBuilder().actionContext?.boardId).toBe("saved-board");
	expect(
		currentBuilder().actionContext?.workflowEvents?.map(
			(event) => event.nodeId,
		),
	).toEqual(["widget-node", "simple-node"]);
	const components = withBinding();
	await act(async () => {
		await currentBuilder().onSave?.(components, {});
	});
	await settle(
		() =>
			boardReads === 2 &&
			builderProps?.actionContext?.workflowEvents?.[0]?.name ===
				"Updated widget schema",
	);
	expect(savedPages).toHaveLength(1);
	expect(savedPages[0].boardId).toBe("saved-board");
	expect(savedPages[0].components?.[0].component.eventHandlers).toEqual({
		entityClicked: [
			{
				name: "workflow_event",
				context: {
					nodeId: "widget-node",
					appId: "app",
					boardId: "saved-board",
				},
			},
		],
	});
	expect(
		(savedPages[0].components?.[0].component as { actionBindings: object })
			.actionBindings,
	).toEqual({});
});

test("auto-save refreshes board metadata after widget event bindings change", async () => {
	await mount();
	const components = withBinding();
	await act(async () => {
		currentBuilder().onChange?.(components, {});
		await new Promise((resolve) => setTimeout(resolve, 2100));
	});
	await settle(
		() =>
			boardReads === 2 &&
			builderProps?.actionContext?.workflowEvents?.[0]?.name ===
				"Updated widget schema",
	);
	expect(savedPages).toHaveLength(1);
	expect(savedPages[0].components?.[0].component.eventHandlers).toEqual(
		components[0].component.eventHandlers,
	);
});

test("re-reads the board on demand and creates a named event node from an untouched catalog template", async () => {
	await mount();
	expect(catalogReads).toBe(0);
	await act(async () =>
		currentBuilder().actionContext?.refreshWorkflowEvents?.(),
	);
	await settle(() => boardReads === 2);

	let created: WorkflowEventInfo | undefined;
	await act(async () => {
		created = await currentBuilder().actionContext?.createWorkflowEvent?.({
			name: "Entity clicked",
			kind: "widget_action",
		});
	});

	expect(catalogReads).toBe(1);
	expect(sentCommands).toHaveLength(1);
	const [command] = sentCommands;
	expect(command.command_type).toBe(ICommandType.AddNode);
	expect(command.current_layer).toBeNull();
	expect(command.node?.name).toBe("events_widget_action");
	expect(command.node?.friendly_name).toBe("Entity clicked");
	expect(command.node?.coordinates).toEqual([
		300,
		120 + WORKFLOW_EVENT_NODE_GAP,
		0,
	]);
	expect(command.node?.id).not.toBe("events_widget_action-template");
	expect(Object.keys(command.node?.pins ?? {})).not.toContain(
		"template-exec-out",
	);
	expect(catalog).toEqual(catalogSnapshot);
	expect(created).toEqual({
		nodeId: "server-node-1",
		name: "Entity clicked",
		kind: "widget_action",
	});
	expect(boardReads).toBe(3);
	await settle(
		() =>
			builderProps?.actionContext?.workflowEvents?.some(
				(event) =>
					event.nodeId === "server-node-1" && event.name === "Entity clicked",
			) ?? false,
	);
});

test("refreshes requested while a board read is running join that read", async () => {
	await mount();
	let releaseBoard = () => {};
	scenario.boardGate = new Promise((resolve) => {
		releaseBoard = resolve;
	});
	await act(async () => {
		currentBuilder().actionContext?.refreshWorkflowEvents?.();
		currentBuilder().actionContext?.refreshWorkflowEvents?.();
	});
	await settle(() => boardReads >= 2);
	expect(boardReads).toBe(2);
	releaseBoard();
	await settle(() => true);
	expect(boardReads).toBe(2);
});

test("a page without a linked board offers no event creation and never reads a board", async () => {
	await mount({ pageBoardId: undefined });
	expect(currentBuilder().actionContext?.boardId).toBeUndefined();
	expect(currentBuilder().actionContext?.createWorkflowEvent).toBeUndefined();
	await act(async () =>
		currentBuilder().actionContext?.refreshWorkflowEvents?.(),
	);
	await openActionsFor("run");
	await settle(() => true);
	expect(boardReads).toBe(0);
	expect(createEventButtons()).toHaveLength(0);
});

test("creating an event whose node is missing from the catalog names the node and runs nothing", async () => {
	await mount({ catalog: [catalogTemplate("events_simple", "Simple Event")] });
	let failure: unknown;
	await act(async () => {
		await currentBuilder()
			.actionContext?.createWorkflowEvent?.({
				name: "Entity clicked",
				kind: "widget_action",
			})
			.catch((error) => {
				failure = error;
			});
	});
	expect(String(failure)).toContain("events_widget_action");
	expect(sentCommands).toHaveLength(0);
	expect(boardReads).toBe(1);
});

test.each([
	["the catalog read", "catalogError"],
	["the board command", "commandError"],
] as const)(
	"a failing %s rejects the create without re-reading the board",
	async (_, failingStep) => {
		const failure = new Error(`${failingStep} from the backend`);
		await mount({ [failingStep]: failure });
		await expect(
			currentBuilder().actionContext?.createWorkflowEvent?.({
				name: "Refresh 2",
				kind: "simple",
			}) ?? Promise.resolve(),
		).rejects.toThrow(failure.message);
		expect(sentCommands).toHaveLength(0);
		expect(boardReads).toBe(1);
	},
);

test("opening a page lifecycle event select re-reads the board and lists only simple events", async () => {
	await mount();
	await click(find("button", "Settings"));
	await mouseDown(find('[role="tab"]', "Behavior"));
	await press(find('[role="combobox"]'), "Enter");
	await settle(() => boardReads === 2);
	const options = [...window.document.querySelectorAll('[role="option"]')].map(
		(option) => option.textContent?.trim(),
	);
	expect(options).toContain("Refresh");
	expect(options).not.toContain("Widget selection");
});

test("a named handler creates a Simple Event, binds it like a manual pick, and refreshes when its select opens", async () => {
	await mount();
	await openActionsFor("run");
	const [handlerButton] = createEventButtons();
	const eventSelect =
		handlerButton.parentElement?.querySelector('[role="combobox"]');
	if (!eventSelect) throw new Error("The workflow event select is missing");
	await press(eventSelect, "Enter");
	await settle(() => boardReads === 2);
	await press(find('[role="listbox"]'), "Escape");

	await click(handlerButton);
	await settle(
		() =>
			liveActions("run").eventHandlers?.click?.[0]?.context?.nodeId ===
			"server-node-1",
	);
	expect(sentCommands.map((command) => command.node?.name)).toEqual([
		"events_simple",
	]);
	expect(sentCommands[0].node?.friendly_name).toBe("Click");
	expect(liveActions("run").eventHandlers?.click).toEqual([
		{
			name: "workflow_event",
			context: {
				nodeId: "server-node-1",
				appId: "app",
				boardId: "saved-board",
			},
		},
	]);
});

test("overlapping creates keep each other's bindings and the legacy default is named after the component", async () => {
	await mount({ holdCommands: true });
	await openActionsFor("run");
	const [handlerButton, legacyButton] = createEventButtons();
	await click(handlerButton);
	await click(legacyButton);
	await settle(() => heldCommands.length === 2);

	releaseHeldCommand();
	await settle(
		() =>
			liveActions("run").eventHandlers?.click?.[0]?.context?.nodeId ===
			"server-node-1",
	);
	releaseHeldCommand();
	await settle(
		() => liveActions("run").actions?.[0]?.context?.nodeId === "server-node-2",
	);

	expect(sentCommands.map((command) => command.node?.friendly_name)).toEqual([
		"Click",
		"run",
	]);
	expect(liveActions("run").eventHandlers?.click?.[0]?.context?.nodeId).toBe(
		"server-node-1",
	);
});

test("a widget instance offers Widget Action Event first and names a wildcard event after the component", async () => {
	await mount();
	await openActionsFor("map");
	const buttons = createEventButtons();
	expect(buttons).toHaveLength(1);
	await pointerDown(buttons[0]);
	const items = [...window.document.querySelectorAll('[role="menuitem"]')];
	expect(items.map((item) => item.textContent?.trim())).toEqual([
		"Widget Action Event",
		"Simple Event",
	]);

	await click(items[0]);
	await settle(
		() =>
			liveActions("map").eventHandlers?.["*"]?.[0]?.context?.nodeId ===
			"server-node-1",
	);
	expect(sentCommands[0].node?.name).toBe("events_widget_action");
	expect(sentCommands[0].node?.friendly_name).toBe("map");
});

test("a failed create leaves the handler untouched and re-enables the button", async () => {
	await mount({ commandError: new Error("board rejected") });
	await openActionsFor("run");
	await click(createEventButtons()[0]);
	await settle(
		() =>
			catalogReads === 1 && !createEventButtons()[0]?.hasAttribute("disabled"),
	);
	expect(sentCommands).toHaveLength(0);
	expect(boardReads).toBe(1);
	expect(liveActions("run").eventHandlers?.click).toEqual(
		unboundWorkflowEvent(),
	);
});

test("a create that finishes after its handler was reset does not bring the handler back", async () => {
	await mount({ holdCommands: true });
	await openActionsFor("run");
	await click(createEventButtons()[0]);
	await settle(() => heldCommands.length === 1);
	await click(find("button", "Use default"));
	await settle(() => liveActions("run").eventHandlers === undefined);

	releaseHeldCommand();
	await settle(
		() =>
			builderProps?.actionContext?.workflowEvents?.some(
				(event) => event.nodeId === "server-node-1",
			) ?? false,
	);
	await settle(() => true);
	expect(liveActions("run").eventHandlers).toBeUndefined();
	expect(liveActions("run").actions).toEqual(unboundWorkflowEvent());
});
