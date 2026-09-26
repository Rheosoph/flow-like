import { afterAll, afterEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type {
	IOfflineTableState,
	IOfflineWritesState,
} from "../../../state/backend-state/offline-writes-state";

const window = new Window({ url: "https://app.test" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	HTMLButtonElement: window.HTMLButtonElement,
	HTMLInputElement: window.HTMLInputElement,
	Element: window.Element,
	Node: window.Node,
	MutationObserver: window.MutationObserver,
	ResizeObserver: window.ResizeObserver,
	Event: window.Event,
	MouseEvent: window.MouseEvent,
	PointerEvent: window.PointerEvent,
	KeyboardEvent: window.KeyboardEvent,
	FocusEvent: window.FocusEvent,
	getComputedStyle: window.getComputedStyle.bind(window),
	IS_REACT_ACT_ENVIRONMENT: true,
});
const { createRoot } = await import("react-dom/client");
const { OfflineTableRow } = await import("./offline-tables-card");

const E31 =
	"Downloading all of table 'orders' needs 3 GiB of offline storage on this device, including room to update the largest fully downloaded table; the limit is 1 GiB.";

const container = document.createElement("div");
document.body.append(container);
const root = createRoot(container);
const calls: unknown[][] = [];
let changed = 0;

function offlineState(): IOfflineWritesState {
	return {
		setPrefetch: async (...args: unknown[]) => {
			calls.push(["setPrefetch", ...args]);
			throw new Error(E31);
		},
		removeTable: async (...args: unknown[]) => {
			calls.push(["removeTable", ...args]);
		},
	} as unknown as IOfflineWritesState;
}

function table(
	overrides: Partial<IOfflineTableState> = {},
): IOfflineTableState {
	return {
		purpose: "storage",
		table: "orders",
		primaryKey: "id",
		prefetch: false,
		status: "ready",
		pendingCount: 0,
		cachedBytes: 0,
		totalBytes: 1024,
		localBytes: 0,
		offlineComplete: false,
		downloading: false,
		waitingForRuns: 0,
		remoteMissing: false,
		...overrides,
	};
}

async function render(state?: IOfflineTableState, name = "orders") {
	await act(async () =>
		root.render(
			<ul>
				<OfflineTableRow
					appId="app-1"
					row={{ purpose: "storage", table: name, state }}
					state={offlineState()}
					enableBlocked={false}
					onEnable={(target) => calls.push(["enable", target])}
					onChanged={() => {
						changed += 1;
					}}
				/>
			</ul>,
		),
	);
}

function switches() {
	return [...container.querySelectorAll<HTMLButtonElement>('[role="switch"]')];
}

afterEach(async () => {
	await act(async () => root.render(null));
	calls.length = 0;
	changed = 0;
});

afterAll(async () => {
	await act(async () => root.unmount());
	window.happyDOM.abort();
});

test("a refused Download everything shows the reason inline and stays off", async () => {
	await render(table());
	const [, prefetch] = switches();
	expect(prefetch.getAttribute("aria-checked")).toBe("false");
	await act(async () => prefetch.click());
	expect(calls).toEqual([["setPrefetch", "app-1", "storage", "orders", true]]);
	expect(container.textContent).toContain(E31);
	expect(switches()[1].getAttribute("aria-checked")).toBe("false");
	expect(changed).toBe(0);
});

test("turning off a table with queued changes explains instead of calling Rust", async () => {
	await render(table({ pendingCount: 2 }));
	await act(async () => switches()[0].click());
	expect(calls).toEqual([]);
	expect(container.textContent).toContain(
		"2 changes are still waiting to sync. Sync or skip them first.",
	);
});

test("an idle table turns off", async () => {
	await render(table());
	await act(async () => switches()[0].click());
	expect(calls).toEqual([["removeTable", "app-1", "storage", "orders"]]);
	expect(changed).toBe(1);
});

test("tables that are not registered yet have no Download everything switch", async () => {
	await render(table({ status: "preparing" }));
	expect(switches()).toHaveLength(1);
	expect(container.textContent).toContain("Preparing…");
});

test("an unconfigured table opens the enable dialog", async () => {
	await render(undefined);
	await act(async () => switches()[0].click());
	expect(calls).toEqual([["enable", { purpose: "storage", table: "orders" }]]);
});

test("unsupported names cannot be turned on", async () => {
	await render(undefined, "orders.v1");
	expect(switches()[0].disabled).toBe(true);
	expect(container.textContent).toContain(
		"Tables with dots or special characters in their name cannot be available offline.",
	);
});
