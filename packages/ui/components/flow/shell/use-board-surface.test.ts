import { afterAll, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import {
	type IBoardSurfaceActions,
	type IBoardSurfaceState,
	useBoardSurface,
} from "./use-board-surface";

const window = new Window({ url: "https://localhost" });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	localStorage: window.localStorage,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
});
// @ts-expect-error — react-dom checks this flag before touching the DOM.
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

interface Probe {
	surface: IBoardSurfaceState;
	actions: IBoardSurfaceActions;
}

let latest: Probe;
let root: Root;
let container: HTMLElement;

function Harness({ mobile }: { mobile: boolean }) {
	latest = useBoardSurface(mobile);
	return null;
}

function mount(mobile = false) {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	act(() => {
		root.render(createElement(Harness, { mobile }));
	});
}

function rerender(mobile: boolean) {
	act(() => {
		root.render(createElement(Harness, { mobile }));
	});
}

const run = (fn: () => void) => act(() => fn());

beforeEach(() => {
	window.localStorage.clear();
});

afterAll(() => {
	act(() => root?.unmount());
});

describe("useBoardSurface", () => {
	test("keeps at most one primary sidebar view open", () => {
		mount();
		run(() => latest.actions.openSidebar("variables"));
		expect(latest.surface.sidebar).toBe("variables");

		run(() => latest.actions.openSidebar("search"));
		expect(latest.surface.sidebar).toBe("search");
	});

	test("toggles a view closed when it is already the open one", () => {
		mount();
		run(() => latest.actions.toggleSidebar("events"));
		expect(latest.surface.sidebar).toBe("events");

		run(() => latest.actions.toggleSidebar("events"));
		expect(latest.surface.sidebar).toBeNull();
	});

	test("closing one region leaves the others untouched", () => {
		mount();
		run(() => {
			latest.actions.openSidebar("variables");
			latest.actions.openPanel("runs");
			latest.actions.openScript();
		});
		expect(latest.surface.sidebar).toBe("variables");
		expect(latest.surface.panel).toBe("runs");
		expect(latest.surface.script).toBe(true);

		run(() => latest.actions.closePanel());
		expect(latest.surface.sidebar).toBe("variables");
		expect(latest.surface.panel).toBeNull();
		expect(latest.surface.script).toBe(true);
	});

	test("a narrow viewport opens the drawer and the docked state together", () => {
		mount(true);
		run(() => latest.actions.toggleSidebar("comments"));
		expect(latest.surface.mobile).toBe("comments");
		expect(latest.surface.sidebar).toBe("comments");

		run(() => latest.actions.toggleSidebar("comments"));
		expect(latest.surface.mobile).toBeNull();
		expect(latest.surface.sidebar).toBeNull();
	});

	test("widening past md docks whatever the drawer was showing", () => {
		mount(true);
		run(() => latest.actions.openScript());
		expect(latest.surface.mobile).toBe("script");

		rerender(false);
		expect(latest.surface.mobile).toBeNull();
		expect(latest.surface.script).toBe(true);
	});

	test("restores the docked layout from storage, never the drawer", () => {
		window.localStorage.setItem(
			"flow-board-shell",
			JSON.stringify({
				sidebar: "variables",
				panel: "traces",
				secondary: "inspector",
				script: true,
			}),
		);
		mount();
		expect(latest.surface.sidebar).toBe("variables");
		expect(latest.surface.panel).toBe("traces");
		expect(latest.surface.secondary).toBe("inspector");
		expect(latest.surface.script).toBe(true);
		expect(latest.surface.mobile).toBeNull();
	});
});
