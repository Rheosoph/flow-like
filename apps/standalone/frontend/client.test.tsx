import { afterAll, afterEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { createContext } from "react";
import type { Root } from "react-dom/client";

const page = new Window({ url: "http://127.0.0.1:8484/ui/" });
Object.assign(page, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window: page,
	document: page.document,
	navigator: page.navigator,
	location: page.location,
	history: page.history,
	localStorage: page.localStorage,
	sessionStorage: page.sessionStorage,
	HTMLElement: page.HTMLElement,
	Element: page.Element,
	Node: page.Node,
	MutationObserver: page.MutationObserver,
	getComputedStyle: page.getComputedStyle.bind(page),
	matchMedia: page.matchMedia.bind(page),
	requestAnimationFrame: page.requestAnimationFrame.bind(page),
	cancelAnimationFrame: page.cancelAnimationFrame.bind(page),
	IS_REACT_ACT_ENVIRONMENT: true,
});

const formProps: Record<string, unknown>[] = [];
const chatProps: Record<string, unknown>[] = [];

mock.module(
	"@flow-like/flow-like-ui/components/interfaces/form-workbench",
	() => ({
		FormWorkbenchInterface: (props: Record<string, unknown>) => {
			formProps.push(props);
			return null;
		},
	}),
);
mock.module(
	"@flow-like/flow-like-ui/components/interfaces/chat-default",
	() => ({
		ChatInterface: (props: Record<string, unknown>) => {
			chatProps.push(props);
			return null;
		},
	}),
);
mock.module(
	"@flow-like/flow-like-ui/components/interfaces/chat-default/message",
	() => ({ ChatFeedbackEnabledContext: createContext(true) }),
);
mock.module(
	"@flow-like/flow-like-ui/components/interfaces/page-interface",
	() => ({ PageInterface: () => null }),
);
mock.module("@flow-like/flow-like-ui/state/execution-engine-context", () => ({
	ExecutionEngineProviderComponent: ({
		children,
	}: { children?: React.ReactNode }) => children,
}));

const bytes = (json: unknown) => [
	...new TextEncoder().encode(JSON.stringify(json)),
];
const event = (id: string, eventType: string, config: unknown = {}) => ({
	id,
	name: id,
	event_type: eventType,
	board_id: "board-1",
	node_id: `node-${id}`,
	event_version: [1, 0, 0],
	config: bytes(config),
	inputs: [],
});

let inventory: unknown;
const realFetch = globalThis.fetch;
globalThis.fetch = (async (input: RequestInfo | URL) => {
	const path = typeof input === "string" ? input : String(input);
	return path === "/services"
		? new Response(JSON.stringify(inventory), { status: 200 })
		: new Response("{}", { status: 404 });
}) as typeof fetch;

const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { default: Service } = await import("./client");

let root: Root | undefined;

async function open(events: unknown[]) {
	inventory = { project_id: "service-project", events };
	const container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
	await act(async () => root?.render(<Service />));
	for (let i = 0; i < 20 && formProps.length + chatProps.length === 0; i++)
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 5));
		});
}

afterEach(async () => {
	await act(async () => root?.unmount());
	root = undefined;
	formProps.length = 0;
	chatProps.length = 0;
	document.body.innerHTML = "";
});

afterAll(() => {
	globalThis.fetch = realFetch;
});

describe("standalone service page", () => {
	test("opens a form as the service host, with its routes and no host toolbar", async () => {
		await open([
			event("intake", "generic_form", { navigate_to_routes: ["/done"] }),
		]);
		const props = formProps.at(-1);
		expect(props).toMatchObject({
			host: "service",
			appId: "service-project",
			event: { id: "intake" },
			config: { navigate_to_routes: ["/done"] },
		});
		expect(typeof props?.onNavigate).toBe("function");
		expect(props?.toolbarRef).toBeUndefined();
	});

	test("opens a quick action as the service host too", async () => {
		await open([event("notify", "quick_action")]);
		expect(formProps.at(-1)).toMatchObject({
			host: "service",
			event: { id: "notify" },
		});
	});

	test("leaves a chat to the chat interface", async () => {
		await open([event("support", "simple_chat")]);
		expect(chatProps.at(-1)).toMatchObject({ event: { id: "support" } });
		expect(formProps).toEqual([]);
	});
});
