import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { createInstance } from "i18next";
import { act } from "react";
import { I18nextProvider } from "react-i18next";
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
const initialState = useLogAggregation.getInitialState();
const i18n = createInstance();
await i18n.init({ lng: "en", resources: {}, initAsync: false });

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
	Object.assign(globalThis, globals);
	TestIntersectionObserver.instances = [];
	useLogAggregation.setState(initialState, true);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
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

async function render(listRuns: IBackendState["boardState"]["listRuns"]) {
	const backend = { boardState: { listRuns }, eventState: {} } as IBackendState;
	await act(async () => {
		root.render(
			<I18nextProvider i18n={i18n}>
				<BackendContext.Provider value={backend}>
					<FlowRuns
						appId="app"
						boardId="board"
						nodes={{}}
						version={[1, 0, 0]}
						executeBoard={async () => {}}
						onVersionChange={() => {}}
						onFocusNode={() => {}}
					/>
				</BackendContext.Provider>
			</I18nextProvider>,
		);
	});
}

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
