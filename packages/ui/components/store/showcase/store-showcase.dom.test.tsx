import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import { IAppType } from "../../../lib/schema/app/app";
import {
	type IAppShowcase,
	emptyShowcase,
} from "../../../lib/schema/app/showcase";

const COLD_IMPORT_TIMEOUT_MS = 60_000;

let window: Window;
let root: Root;
let host: HTMLElement;
let restoreGlobals: () => void;

beforeEach(async () => {
	window = new Window({ url: "https://app.flow-like.com/store?id=app-1" });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLAnchorElement: window.HTMLAnchorElement,
		HTMLImageElement: window.HTMLImageElement,
		SVGElement: window.SVGElement,
		Element: window.Element,
		Text: window.Text,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		ShadowRoot: window.ShadowRoot,
		DOMRect: window.DOMRect,
		Range: window.Range,
		Selection: window.Selection,
		HTMLCollection: window.HTMLCollection,
		NodeList: window.NodeList,
		Node: window.Node,
		MutationObserver: window.MutationObserver,
		IntersectionObserver: window.IntersectionObserver,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		KeyboardEvent: window.KeyboardEvent,
		FocusEvent: window.FocusEvent,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		NodeFilter: window.NodeFilter,
		ResizeObserver: window.ResizeObserver,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		getComputedStyle: window.getComputedStyle.bind(window),
		matchMedia: window.matchMedia.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	Object.assign(window, { SyntaxError, TypeError });
	restoreGlobals = () => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};

	const { createRoot } = await import("react-dom/client");
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
});

afterEach(async () => {
	const { act } = await import("react");
	await act(() => root.unmount());
	await window.happyDOM.abort();
	restoreGlobals();
});

const SECRET = "ada@example.com";

function fullShowcase(): IAppShowcase {
	return {
		...emptyShowcase("app-1"),
		map: true,
		tables: [
			{
				name: "customers",
				rows: 1200,
				columns: [
					{
						name: "name",
						data_type: "Utf8",
						family: "text",
						nullable: false,
						personal: false,
					},
					{
						name: "email",
						data_type: "Utf8",
						family: "text",
						nullable: false,
						personal: true,
					},
				],
				sample: {
					rows: [
						["Ada", SECRET],
						["Grace", "grace@example.com"],
					],
				},
			},
			{ name: "orders", rows: 40 },
		],
		events: [
			{
				id: "ev-chat",
				name: "Ask the app",
				event_type: "simple_chat",
				board_id: "flow-1",
			},
			{
				id: "ev-api",
				name: "Orders API",
				event_type: "api",
				board_id: "flow-hidden",
				technical: { method: "POST", path: "/orders" },
			},
		],
		flows: [
			{
				id: "flow-1",
				name: "Answer",
				description: "",
				node_count: 12,
				tables: ["customers", "not-shown"],
				pages: ["page-1"],
			},
		],
		pages: [
			{
				id: "page-1",
				name: "Dashboard",
				route: "/dash",
				outline: { t: "column", c: [{ t: "text" }, { t: "chart" }] },
				component_count: 3,
			},
		],
		widgets: [{ id: "w-1", name: "Open orders" }],
		templates: [{ id: "tpl-1", name: "Starter" }],
		hood: { execution_mode: "Any", refuse_bots: false, allow_forking: true },
		activity: {
			runs: Array.from({ length: 30 }, (_, i) => ({
				day: `2026-11-${String(i + 1).padStart(2, "0")}`,
				runs: i,
			})),
			versions: [{ version: "1.0.0", date: null, notes: "First release" }],
		},
	};
}

async function render(showcase: IAppShowcase) {
	const { act } = await import("react");
	const { StoreShowcase } = await import("./store-showcase");
	await act(async () =>
		root.render(
			<StoreShowcase
				app={{ app_type: IAppType.DataFocus, price: 0 } as never}
				showcase={showcase}
			/>,
		),
	);
}

const query = (selector: string) =>
	Array.from(window.document.body.querySelectorAll(selector));

describe("store showcase", () => {
	test(
		"renders the sections in the data app order",
		async () => {
			await render(fullShowcase());
			const order = query("[data-showcase-section]").map((el) =>
				el.getAttribute("data-showcase-section"),
			);
			expect(order).toEqual([
				"data",
				"map",
				"triggers",
				"pages",
				"widgets",
				"templates",
				"hood",
				"activity",
			]);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test("masks personal columns in sample rows", async () => {
		const { act } = await import("react");
		await render(fullShowcase());
		const card = query("button").find((b) =>
			b.textContent?.includes("customers"),
		);
		await act(async () => (card as unknown as HTMLElement).click());
		const tab = query("[role=tab]").find((b) =>
			b.textContent?.includes("Sample rows"),
		) as unknown as HTMLElement;
		await act(async () => {
			tab.dispatchEvent(
				new window.MouseEvent("mousedown", {
					bubbles: true,
					button: 0,
				}) as unknown as Event,
			);
		});
		expect(query("td[data-masked]").length).toBe(2);
		expect(window.document.body.textContent).toContain("Ada");
		expect(window.document.body.textContent).not.toContain(SECRET);
	});

	test("an empty showcase renders nothing", async () => {
		await render(emptyShowcase("app-1"));
		expect(host.innerHTML).toBe("");
	});

	test("the map only draws edges between shown nodes", async () => {
		await render(fullShowcase());
		const edges = query("[data-edge]")
			.map((el) => el.getAttribute("data-edge"))
			.sort();
		expect(edges).toEqual([
			"event:ev-chat>flow:flow-1",
			"flow:flow-1>page:page-1",
			"flow:flow-1>table:customers",
		]);
	});

	test("with flows hidden, triggers link straight to their tables and pages", async () => {
		const showcase = fullShowcase();
		await render({
			...showcase,
			flows: [],
			events: [
				{
					id: "ev-chat",
					name: "Ask the app",
					event_type: "simple_chat",
					tables: ["customers"],
					pages: ["page-1"],
				},
			],
		});
		const edges = query("[data-edge]")
			.map((el) => el.getAttribute("data-edge"))
			.sort();
		expect(edges).toEqual([
			"event:ev-chat>page:page-1",
			"event:ev-chat>table:customers",
		]);
	});

	test("hides the map with fewer than two lanes", async () => {
		const showcase = fullShowcase();
		await render({
			...showcase,
			events: [],
			flows: [],
			pages: [],
			widgets: [],
			tables: [showcase.tables[0]],
		});
		expect(query("[data-showcase-section=map]").length).toBe(0);
	});
});
