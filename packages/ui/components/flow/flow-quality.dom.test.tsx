import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import type {
	IBoardQualityReport,
	IQualityFinding,
} from "../../lib/board-quality";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });

// bun keeps globals and module mocks for every later file in the process, so both are
// captured first and put back in afterAll.
const globalDescriptors = [
	"window",
	"document",
	"navigator",
	"localStorage",
	"HTMLElement",
	"Element",
	"Node",
	"MutationObserver",
	"HTMLButtonElement",
	"SVGElement",
	"Event",
	"CustomEvent",
	"MouseEvent",
	"PointerEvent",
	"getComputedStyle",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"ResizeObserver",
	"IS_REACT_ACT_ENVIRONMENT",
].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
const actualLocales = { ...(await import("@flow-like/locales")) };

function installDomGlobals() {
	Object.assign(globalThis, {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		Element: window.Element,
		Node: window.Node,
		MutationObserver: window.MutationObserver,
		HTMLButtonElement: window.HTMLButtonElement,
		SVGElement: window.SVGElement,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: (cb: FrameRequestCallback) =>
			setTimeout(() => cb(0), 0),
		cancelAnimationFrame: (id: number) => clearTimeout(id),
		ResizeObserver: class {
			observe() {}
			unobserve() {}
			disconnect() {}
		},
	});
}
installDomGlobals();

// @ts-expect-error — react-dom checks this flag before touching the DOM.
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const translate = (
	_key: string,
	fallback: string | Record<string, unknown>,
	opts?: Record<string, unknown>,
) => {
	const options =
		typeof fallback === "object"
			? fallback
			: (opts ?? ({} as Record<string, unknown>));
	const template =
		typeof fallback === "string"
			? fallback
			: ((options.count === 1
					? options.defaultValue_one
					: (options.defaultValue_other ?? options.defaultValue)) as string);
	return (template ?? "").replace(/\{\{(\w+)\}\}/g, (_, key: string) =>
		String(options[key] ?? ""),
	);
};
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate }),
}));

const { createRoot } = await import("react-dom/client");
const { FlowQuality } = await import("./flow-quality");
const { FlowNodeQualityBadge } = await import(
	"./flow-node/flow-node-quality-badge"
);
const { useBoardQualityStore } = await import(
	"../../state/board-quality-state"
);
const { analyzeBoardQuality } = await import("../../lib/board-quality");

beforeAll(() => installDomGlobals());

const roots: ReturnType<typeof createRoot>[] = [];

function render(element: React.ReactElement): HTMLElement {
	const container = window.document.createElement(
		"div",
	) as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	const root = createRoot(container);
	roots.push(root);
	act(() => root.render(element));
	return container;
}

afterAll(async () => {
	await act(async () => {
		for (const root of roots) root.unmount();
	});
	mock.module("@flow-like/locales", () => actualLocales);
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

function finding(
	overrides: Partial<IQualityFinding> & Pick<IQualityFinding, "id">,
): IQualityFinding {
	return {
		rule: "dead-code",
		severity: "warning",
		target: {
			kind: "node",
			id: overrides.id,
			name: overrides.id,
			layerId: null,
		},
		detail: { kind: "unreachable" },
		...overrides,
	};
}

function report(findings: IQualityFinding[]): IBoardQualityReport {
	const counts = { error: 0, warning: 0, info: 0 };
	for (const entry of findings) counts[entry.severity] += 1;
	const marks: IBoardQualityReport["marks"] = {};
	for (const entry of findings) {
		marks[entry.target.id] = {
			severity: entry.severity,
			counts: { error: 0, warning: 0, info: 0, [entry.severity]: 1 },
			findings: [entry],
			nested: 0,
			signature: entry.id,
		};
	}
	return { findings, counts, marks, nodeCount: 3, durationMs: 1 };
}

function click(element: Element | null | undefined) {
	if (!element) throw new Error("element not found");
	act(() => {
		element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
	});
}

describe("FlowQuality", () => {
	test("renders the empty state until a report carries findings", () => {
		useBoardQualityStore.getState().clear("empty");
		const container = render(
			createElement(FlowQuality, {
				boardId: "empty",
				board: undefined,
				onFocusNode: () => {},
				onOpenVariables: () => {},
			}),
		);
		expect(container.textContent).toContain("No issues found");
	});

	test("groups findings by rule, focuses nodes, and routes variables to the variables view", () => {
		const focused: string[] = [];
		let variablesOpened = 0;
		useBoardQualityStore.getState().setReport(
			"b1",
			report([
				finding({ id: "orphan" }),
				finding({
					id: "secret",
					rule: "hardcoded-secret",
					severity: "error",
					target: {
						kind: "variable",
						id: "var",
						name: "apiKey",
						layerId: null,
					},
					detail: { kind: "variable-not-secret" },
				}),
			]),
		);
		const container = render(
			createElement(FlowQuality, {
				boardId: "b1",
				board: undefined,
				onFocusNode: (id) => focused.push(id),
				onOpenVariables: () => {
					variablesOpened += 1;
				},
			}),
		);
		expect(container.textContent).toContain("Hard-coded secret");
		expect(container.textContent).toContain("Dead code");
		expect(container.textContent).toContain("No entry point reaches this node");

		const rows = Array.from(container.querySelectorAll("li > button"));
		click(rows.find((row) => row.textContent?.includes("orphan")));
		expect(focused).toEqual(["orphan"]);
		click(rows.find((row) => row.textContent?.includes("apiKey")));
		expect(variablesOpened).toBe(1);
	});

	test("the severity chips filter rows and the eye toggles inline marks", () => {
		useBoardQualityStore.getState().setReport(
			"b2",
			report([
				finding({ id: "w1" }),
				finding({
					id: "i1",
					severity: "info",
					detail: { kind: "unused-result" },
				}),
			]),
		);
		const container = render(
			createElement(FlowQuality, {
				boardId: "b2",
				board: undefined,
				onFocusNode: () => {},
				onOpenVariables: () => {},
			}),
		);
		expect(container.textContent).toContain("i1");
		click(container.querySelector('button[title="Hints"]'));
		expect(container.textContent).not.toContain("i1");
		expect(container.textContent).toContain("w1");
		click(container.querySelector('button[title="Hints"]'));

		const before = useBoardQualityStore.getState().inline;
		const eye = Array.from(container.querySelectorAll("button")).find(
			(button) =>
				button.getAttribute("aria-pressed") === String(before) &&
				!button.hasAttribute("title"),
		);
		click(eye);
		expect(useBoardQualityStore.getState().inline).toBe(!before);
		useBoardQualityStore.getState().setInline(true);
	});
});

describe("FlowNodeQualityBadge", () => {
	test("renders nothing without a mark and a counted badge with one", () => {
		useBoardQualityStore.getState().setInline(true);
		useBoardQualityStore
			.getState()
			.setReport("b3", report([finding({ id: "flagged" })]));
		const clean = render(
			createElement(FlowNodeQualityBadge, { boardId: "b3", targetId: "clean" }),
		);
		expect(clean.innerHTML).toBe("");

		const flagged = render(
			createElement(FlowNodeQualityBadge, {
				boardId: "b3",
				targetId: "flagged",
			}),
		);
		expect(flagged.querySelector("svg")).not.toBeNull();
		expect(flagged.firstElementChild?.getAttribute("title")).toContain(
			"No entry point reaches this node",
		);

		act(() => useBoardQualityStore.getState().setInline(false));
		expect(flagged.innerHTML).toBe("");
		useBoardQualityStore.getState().setInline(true);
	});

	test("a real report round-trips through the store into a badge", () => {
		const boardReport = analyzeBoardQuality({
			id: "b4",
			nodes: {
				lonely: {
					id: "lonely",
					name: "http_request",
					friendly_name: "Lonely",
					description: "",
					category: "",
					pins: {
						in: {
							id: "in",
							name: "exec_in",
							friendly_name: "In",
							description: "",
							pin_type: "Input",
							data_type: "Execution",
							value_type: "Normal",
							depends_on: [],
							connected_to: [],
							index: 0,
						},
					},
				},
			},
			layers: {},
			variables: {},
		} as never);
		useBoardQualityStore.getState().setReport("b4", boardReport);
		const container = render(
			createElement(FlowNodeQualityBadge, {
				boardId: "b4",
				targetId: "lonely",
			}),
		);
		expect(container.querySelector("svg")).not.toBeNull();
	});
});
