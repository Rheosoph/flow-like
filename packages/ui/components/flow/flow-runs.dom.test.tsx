import {
	afterAll,
	afterEach,
	beforeEach,
	expect,
	mock,
	spyOn,
	test,
} from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { createInstance } from "i18next";
import { act } from "react";
import { I18nextProvider } from "react-i18next";
import type { INode } from "../../lib/schema/flow/board";
import type { IEvent } from "../../lib/schema/flow/event";
import type { ILogMetadata } from "../../lib/schema/flow/log-metadata";
import type { IBackendState } from "../../state/backend-state";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });

class TestIntersectionObserver {
	static instances: TestIntersectionObserver[] = [];
	readonly targets = new Set<Element>();
	readonly root: Element | Document | null;
	readonly rootMargin: string;
	readonly thresholds = [0];

	constructor(
		private callback: IntersectionObserverCallback,
		options: IntersectionObserverInit = {},
	) {
		this.root = options.root ?? null;
		this.rootMargin = options.rootMargin ?? "0px";
		TestIntersectionObserver.instances.push(this);
	}

	observe(target: Element) {
		this.targets.add(target);
	}
	unobserve(target: Element) {
		this.targets.delete(target);
	}
	disconnect() {
		this.targets.clear();
	}
	takeRecords() {
		return [];
	}
	intersect() {
		this.callback(
			Array.from(this.targets, (target) => ({
				target,
				isIntersecting: true,
			})) as IntersectionObserverEntry[],
			this as unknown as IntersectionObserver,
		);
	}
}

const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	DocumentFragment: window.DocumentFragment,
	MutationObserver: window.MutationObserver,
	HTMLButtonElement: window.HTMLButtonElement,
	SVGElement: window.SVGElement,
	Event: window.Event,
	CustomEvent: window.CustomEvent,
	MouseEvent: window.MouseEvent,
	PointerEvent: window.PointerEvent,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: window.requestAnimationFrame.bind(window),
	cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
	IntersectionObserver: TestIntersectionObserver,
	IS_REACT_ACT_ENVIRONMENT: true,
};
const globalDescriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);

const { createRoot } = await import("react-dom/client");
const { BackendContext } = await import("../../state/backend-state");
const { useLogAggregation } = await import("../../state/log-aggregation-state");
const { FlowRuns } = await import("./flow-runs");
const { toast } = await import("sonner");
const initialState = useLogAggregation.getInitialState();
const i18n = createInstance();
await i18n.init({ lng: "en", resources: {}, initAsync: false });

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let queryClient: QueryClient;

beforeEach(() => {
	Object.assign(globalThis, globals);
	TestIntersectionObserver.instances = [];
	useLogAggregation.setState(initialState, true);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false, gcTime: 0 } },
	});
});

afterEach(async () => {
	await act(async () => root.unmount());
	queryClient.clear();
	container.remove();
	useLogAggregation.setState(initialState, true);
});

afterAll(async () => {
	await window.happyDOM.close();
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

function runAt(index: number): ILogMetadata {
	return {
		app_id: "app",
		board_id: "board",
		run_id: `run-${index}`,
		event_id: "event",
		node_id: "node",
		start: (1000 - index) * 1_000_000,
		end: (1000 - index) * 1_000_000,
		version: `run-version-${index}`,
		log_level: 1,
		payload: [],
	};
}

async function render(
	listRuns: IBackendState["boardState"]["listRuns"],
	options: {
		appId?: string;
		getEvents?: IBackendState["eventState"]["getEvents"];
		getCorpusPayload?: IBackendState["eventState"]["getCorpusPayload"];
		executeBoard?: (node: INode, payload?: object) => Promise<void>;
		nodes?: Record<string, INode>;
	} = {},
) {
	const backend = {
		boardState: { listRuns },
		eventState: {
			getEvents: options.getEvents ?? (async () => []),
			getCorpusPayload: options.getCorpusPayload,
		},
	} as IBackendState;
	await act(async () => {
		root.render(
			<QueryClientProvider client={queryClient}>
				<I18nextProvider i18n={i18n}>
					<BackendContext.Provider value={backend}>
						<FlowRuns
							appId={options.appId ?? "app"}
							boardId="board"
							nodes={options.nodes ?? {}}
							version={[1, 0, 0]}
							executeBoard={options.executeBoard ?? (async () => {})}
							onVersionChange={() => {}}
							onFocusNode={() => {}}
						/>
					</BackendContext.Provider>
				</I18nextProvider>
			</QueryClientProvider>,
		);
	});
}

async function flushQueries() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

test("legacy cron runs show the existing Event name when their node ID contains the Event ID", async () => {
	const getEvents = mock(async () => [
		{ id: "event", name: "Nightly import", event_type: "cron" } as IEvent,
	]);
	await render(
		async () => [
			{ ...runAt(0), node_id: "event", is_remote: true },
			{ ...runAt(1), node_id: "event", is_remote: true },
		],
		{ getEvents },
	);
	await flushQueries();

	expect(container.textContent?.match(/Nightly import/g)).toHaveLength(2);
	expect(container.textContent).not.toContain("Deleted Event");
	expect(getEvents).toHaveBeenCalledTimes(1);
	expect(getEvents).toHaveBeenCalledWith("app", true);
});

test("runs with a known node keep its name without loading Events", async () => {
	const getEvents = mock(async () => []);
	const node = { id: "node", friendly_name: "Schedule start" } as INode;
	await render(async () => [runAt(0)], {
		getEvents,
		nodes: { [node.id]: node },
	});

	expect(container.textContent).toContain("Schedule start");
	expect(container.textContent).not.toContain("Deleted Event");
	expect(getEvents).not.toHaveBeenCalled();
});

test("legacy runs retain the deleted label when their Event no longer exists", async () => {
	await render(async () => [
		{ ...runAt(0), node_id: "event", is_remote: true },
	]);
	await flushQueries();

	expect(container.textContent).toContain("Deleted Event");
});

test("a failed Event lookup does not label an existing cron Event as deleted", async () => {
	const consoleError = spyOn(console, "error").mockImplementation(() => {});
	try {
		await render(
			async () => [{ ...runAt(0), node_id: "event", is_remote: true }],
			{
				getEvents: async () => {
					throw new Error("Connection lost");
				},
			},
		);
		await flushQueries();

		expect(container.textContent).not.toContain("Deleted Event");
		expect(container.querySelector("small.leading-none")?.textContent).toBe(
			"Event",
		);
	} finally {
		consoleError.mockRestore();
	}
});

test("an Event lookup pending for a previous app cannot rename the current app's runs", async () => {
	let resolveEvents!: (events: IEvent[]) => void;
	const oldEvents = new Promise<IEvent[]>((resolve) => {
		resolveEvents = resolve;
	});
	const getEvents = mock(async (appId: string) =>
		appId === "app" ? oldEvents : [],
	);
	const listRuns = async () => [
		{ ...runAt(0), node_id: "event", is_remote: true },
	];
	await render(listRuns, { getEvents });
	expect(container.textContent).not.toContain("Deleted Event");
	await render(listRuns, { appId: "other-app", getEvents });
	await flushQueries();
	await act(async () => {
		resolveEvents([{ id: "event", name: "Previous app's schedule" } as IEvent]);
	});
	await flushQueries();

	expect(container.textContent).toContain("Deleted Event");
	expect(container.textContent).not.toContain("Previous app's schedule");
});

async function rerunFirst() {
	const actions = container.querySelector<HTMLButtonElement>(
		'button[aria-label="Log Actions"]',
	);
	expect(actions).not.toBeNull();
	await act(async () => {
		actions?.dispatchEvent(
			new PointerEvent("pointerdown", {
				bubbles: true,
				pointerType: "mouse",
				button: 0,
			}),
		);
	});
	const rerun = Array.from(
		document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
	).find((item) => item.textContent === "Re-Run");
	expect(rerun).toBeDefined();
	await act(async () => rerun?.click());
}

test("a run's stored failure reason shows as one line with the full text as its title", async () => {
	const reason = "Run exceeded grace period without completion event";
	await render(async () => [
		{ ...runAt(0), log_level: 4, error_message: reason },
		{ ...runAt(1), error_message: null },
	]);

	const lines = container.querySelectorAll<HTMLElement>(`[title="${reason}"]`);
	expect(lines).toHaveLength(1);
	expect(lines[0].textContent).toBe(reason);
	expect(lines[0].querySelector(".truncate")?.textContent).toBe(reason);
	expect(container.textContent).toContain("run-version-1");
});

test("rerunning a remote event reads its hub payload and resolved start node", async () => {
	const run = { ...runAt(0), is_remote: true };
	const node = { id: "resolved-node", friendly_name: "Inbound email" } as INode;
	const payload = { subject: "Jenny Test" };
	const getCorpusPayload = mock(async () => ({
		run_id: run.run_id,
		board_id: "board",
		node_id: node.id,
		payload,
	}));
	const executeBoard = mock(async (_node: INode, _payload?: object) => {});
	await render(async () => [run], {
		getCorpusPayload,
		executeBoard,
		nodes: { [node.id]: node },
	});

	await rerunFirst();

	expect(getCorpusPayload).toHaveBeenCalledWith(
		"app",
		"event",
		run.run_id,
		"remote",
	);
	expect(executeBoard).toHaveBeenCalledWith(node, payload);
});

test("rerun displays a structured native error without starting another run", async () => {
	const run = { ...runAt(0), is_remote: true };
	const errorToast = spyOn(toast, "error").mockImplementation(() => "error");
	try {
		const getCorpusPayload = mock(async () => {
			throw { error: "Run not found for this event" };
		});
		const executeBoard = mock(async (_node: INode, _payload?: object) => {});
		await render(async () => [run], { getCorpusPayload, executeBoard });

		await rerunFirst();

		expect(errorToast).toHaveBeenCalledWith("Run not found for this event");
		expect(executeBoard).not.toHaveBeenCalled();
	} finally {
		errorToast.mockRestore();
	}
});

function activeObserver() {
	const observer = TestIntersectionObserver.instances.find(
		(instance) => instance.targets.size > 0,
	);
	if (!observer)
		throw new Error("Run list did not observe its bottom sentinel");
	return observer;
}

test("reaching the run list bottom appends the next page and stops when exhausted", async () => {
	const runs = Array.from({ length: 103 }, (_, index) => runAt(index));
	const listRuns = mock<IBackendState["boardState"]["listRuns"]>(
		async (
			_app,
			_board,
			_node,
			_from,
			_to,
			_status,
			_last,
			offset = 0,
			limit = 100,
		) => runs.slice(offset, offset + limit),
	);
	await render(listRuns);
	expect(useLogAggregation.getState().currentLogs).toHaveLength(100);
	expect(container.textContent).toContain("run-version-0");
	expect(container.textContent).not.toContain("run-version-100");
	const observer = activeObserver();
	const sentinel = [...observer.targets][0];
	expect(observer.root).toBe(sentinel.parentElement);
	expect(observer.rootMargin).toBe("200px");
	expect((observer.root as Element).classList.contains("overflow-y-auto")).toBe(
		true,
	);

	await act(async () => observer.intersect());
	expect(listRuns.mock.calls.filter((call) => call[7] === 100)).toHaveLength(1);
	expect(useLogAggregation.getState().currentLogs).toHaveLength(103);
	expect(container.textContent).toContain("run-version-0");
	expect(container.textContent).toContain("run-version-102");
	expect(useLogAggregation.getState().hasMore).toBe(false);
	expect(container.textContent).not.toContain("Load more");
	expect(
		TestIntersectionObserver.instances.every(
			(instance) => instance.targets.size === 0,
		),
	).toBe(true);
	const requests = listRuns.mock.calls.length;
	await act(async () => observer.intersect());
	expect(listRuns.mock.calls).toHaveLength(requests);
});

test("a failed next page keeps loaded runs and offers a manual retry", async () => {
	const runs = Array.from({ length: 101 }, (_, index) => runAt(index));
	let failed = false;
	const listRuns = mock<IBackendState["boardState"]["listRuns"]>(
		async (
			_app,
			_board,
			_node,
			_from,
			_to,
			_status,
			_last,
			offset = 0,
			limit = 100,
		) => {
			if (offset === 100 && !failed) {
				failed = true;
				throw new Error("Connection lost");
			}
			return runs.slice(offset, offset + limit);
		},
	);
	await render(listRuns);
	await act(async () => activeObserver().intersect());
	expect(useLogAggregation.getState().currentLogs).toHaveLength(100);
	expect(useLogAggregation.getState().loadMoreFailed).toBe(true);
	expect(
		TestIntersectionObserver.instances.every(
			(instance) => instance.targets.size === 0,
		),
	).toBe(true);
	const retry = Array.from(container.querySelectorAll("button")).find(
		(button) => button.textContent === "Retry",
	);
	expect(retry).toBeDefined();
	await act(async () => retry?.click());
	expect(listRuns.mock.calls.filter((call) => call[7] === 100)).toHaveLength(2);
	expect(container.textContent).toContain("run-version-100");
	expect(useLogAggregation.getState().currentLogs).toHaveLength(101);
	expect(useLogAggregation.getState().hasMore).toBe(false);
	expect(useLogAggregation.getState().loadMoreFailed).toBe(false);
});
