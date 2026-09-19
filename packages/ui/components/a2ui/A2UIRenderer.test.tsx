import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { ComponentProps } from "./ComponentRegistry";
import type { BoundValue, Surface, SurfaceComponent } from "./types";

// No module mocks: bun keeps a mocked module registered for every file that
// runs later in the same process, so the real registry and provider are used.
const [
	{ A2UIRenderer },
	{ registerComponent },
	{ useData },
	{ A2UIColumn },
	{ appGlobalState, pageLocalState },
	uiState,
	{ AppRouterContext },
	{ PathnameContext },
] = await Promise.all([
	import("./A2UIRenderer"),
	import("./ComponentRegistry"),
	import("./DataContext"),
	import("./layout/Column"),
	import("../../lib/idb-storage"),
	import("../../db/ui-state-db"),
	import("next/dist/shared/lib/app-router-context.shared-runtime"),
	import("next/dist/shared/lib/hooks-client-context.shared-runtime"),
]);

/** Render counts per component id, across every renderer used by the fixtures. */
const renders: Record<string, number> = {};
const count = (id: string) => {
	renders[id] = (renders[id] ?? 0) + 1;
};

function Probe({ component, componentId, elementRef }: ComponentProps) {
	const { resolve } = useData();
	count(componentId);
	const text = String(
		resolve((component as unknown as { text: BoundValue }).text) ?? "",
	);
	return (
		<span ref={elementRef} data-probe={componentId}>
			{text}
		</span>
	);
}

function CountingColumn(props: ComponentProps) {
	count(props.componentId);
	return <A2UIColumn {...(props as Parameters<typeof A2UIColumn>[0])} />;
}

// Test-only types; the real column renderer is reused so child resolution
// (explicit lists, templates, scopes) is the production code path.
registerComponent("testProbe", Probe);
registerComponent("testColumn", CountingColumn);

const probe = (
	id: string,
	text: string | BoundValue,
	extra: Record<string, unknown> = {},
): SurfaceComponent => ({
	id,
	component: {
		type: "testProbe",
		text: typeof text === "string" ? { literalString: text } : text,
		...extra,
	} as unknown as SurfaceComponent["component"],
});

const column = (
	id: string,
	children: SurfaceComponent["component"]["children"],
): SurfaceComponent => ({
	id,
	component: {
		type: "testColumn",
		children,
	} as unknown as SurfaceComponent["component"],
});

function surfaceOf(
	components: SurfaceComponent[],
	extra: Partial<Surface> = {},
): Surface {
	return {
		id: "s1",
		rootComponentId: "root",
		components: Object.fromEntries(components.map((c) => [c.id, c])),
		...extra,
	};
}

/** Structural sharing exactly as the shared reducer does it. */
function withComponent(surface: Surface, component: SurfaceComponent): Surface {
	return {
		...surface,
		components: { ...surface.components, [component.id]: component },
	};
}

let root: Root | undefined;
let host: HTMLElement | undefined;
const cleanup: (() => void)[] = [];
const router = {} as never;

function mount(surface: Surface) {
	const window = new Window({ url: "https://local/use" });
	Object.assign(window, { SyntaxError });
	const globals = {
		window,
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const previous = Object.fromEntries(
		Object.keys(globals).map((key) => [
			key,
			Object.getOwnPropertyDescriptor(globalThis, key),
		]),
	);
	Object.assign(globalThis, globals);
	cleanup.push(() => {
		for (const [key, descriptor] of Object.entries(previous)) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	});
	const spies = [
		spyOn(appGlobalState, "getAll").mockResolvedValue({}),
		spyOn(pageLocalState, "getAll").mockResolvedValue({}),
		spyOn(uiState.uiElementValues, "getAll").mockResolvedValue({}),
		spyOn(uiState, "pruneElementValues").mockResolvedValue(),
	];
	cleanup.push(() => {
		for (const spy of spies) spy.mockRestore();
	});
	const element = window.document.createElement("div");
	window.document.body.append(element);
	host = element as unknown as HTMLElement;
	root = createRoot(host);
	return render(surface);
}

async function render(surface: Surface) {
	await act(async () => {
		root?.render(
			<AppRouterContext.Provider value={router}>
				<PathnameContext.Provider value="/use">
					<A2UIRenderer surface={surface} isPreviewMode />
				</PathnameContext.Provider>
			</AppRouterContext.Provider>,
		);
	});
}

const probeText = (id: string) =>
	host?.querySelector(`[data-probe="${id}"]`)?.textContent ?? null;

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	host = undefined;
	for (const restore of cleanup.reverse()) restore();
	cleanup.length = 0;
	for (const key of Object.keys(renders)) delete renders[key];
});

describe("A2UIRenderer per-node updates", () => {
	test("a changed component re-renders alone", async () => {
		let surface = surfaceOf([
			column("root", { explicitList: ["a", "b"] }),
			probe("a", "one"),
			probe("b", "two"),
		]);
		await mount(surface);
		expect(probeText("a")).toBe("one");
		const before = { ...renders };

		surface = withComponent(surface, probe("a", "changed"));
		await render(surface);

		expect(probeText("a")).toBe("changed");
		expect(probeText("b")).toBe("two");
		expect(renders.a).toBe(before.a + 1);
		expect(renders.b).toBe(before.b);
		expect(renders.root).toBe(before.root);
	});

	test("a deep change leaves parents, uncles and siblings untouched", async () => {
		let surface = surfaceOf([
			column("root", { explicitList: ["inner", "b"] }),
			column("inner", { explicitList: ["a1", "a2"] }),
			probe("a1", "1"),
			probe("a2", "2"),
			probe("b", "b"),
		]);
		await mount(surface);
		const before = { ...renders };

		surface = withComponent(surface, probe("a2", "2!"));
		await render(surface);

		expect(probeText("a2")).toBe("2!");
		expect(renders.a2).toBe(before.a2 + 1);
		expect(renders.a1).toBe(before.a1);
		expect(renders.inner).toBe(before.inner);
		expect(renders.root).toBe(before.root);
		expect(renders.b).toBe(before.b);
	});

	test("a parent's child list mounts and unmounts without re-rendering the rest", async () => {
		let surface = surfaceOf([
			column("root", { explicitList: ["a", "b"] }),
			probe("a", "one"),
			probe("b", "two"),
			probe("c", "three"),
		]);
		await mount(surface);
		expect(probeText("c")).toBeNull();
		const before = { ...renders };

		surface = withComponent(
			surface,
			column("root", { explicitList: ["a", "b", "c"] }),
		);
		await render(surface);
		expect(probeText("c")).toBe("three");
		expect(renders.a).toBe(before.a);
		expect(renders.b).toBe(before.b);

		surface = withComponent(
			surface,
			column("root", { explicitList: ["b", "c"] }),
		);
		await render(surface);
		expect(probeText("a")).toBeNull();
		expect(probeText("b")).toBe("two");
		expect(renders.b).toBe(before.b);
	});

	test("several updates batched into one commit all land", async () => {
		let surface = surfaceOf([
			column("root", { explicitList: ["a", "b"] }),
			probe("a", "one"),
			probe("b", "two"),
		]);
		await mount(surface);

		await act(async () => {
			surface = withComponent(surface, probe("a", "a2"));
			root?.render(
				<AppRouterContext.Provider value={router}>
					<PathnameContext.Provider value="/use">
						<A2UIRenderer surface={surface} isPreviewMode />
					</PathnameContext.Provider>
				</AppRouterContext.Provider>,
			);
			surface = withComponent(surface, probe("b", "b2"));
			root?.render(
				<AppRouterContext.Provider value={router}>
					<PathnameContext.Provider value="/use">
						<A2UIRenderer surface={surface} isPreviewMode />
					</PathnameContext.Provider>
				</AppRouterContext.Provider>,
			);
		});

		expect(probeText("a")).toBe("a2");
		expect(probeText("b")).toBe("b2");
	});

	test("hidden toggles a node off and back on", async () => {
		let surface = surfaceOf([
			column("root", { explicitList: ["a", "b"] }),
			probe("a", "one"),
			probe("b", "two"),
		]);
		await mount(surface);

		surface = withComponent(
			surface,
			probe("b", "two", { hidden: { literalBool: true } }),
		);
		await render(surface);
		expect(probeText("b")).toBeNull();
		expect(probeText("a")).toBe("one");

		surface = withComponent(surface, probe("b", "two"));
		await render(surface);
		expect(probeText("b")).toBe("two");
	});

	test("data model updates reach bound nodes", async () => {
		let surface = surfaceOf(
			[
				column("root", { explicitList: ["a", "b"] }),
				probe("a", "static"),
				probe("b", { path: "/x" }),
			],
			{ dataModel: [{ path: "/x", value: "one" }] },
		);
		await mount(surface);
		expect(probeText("b")).toBe("one");

		surface = { ...surface, dataModel: [{ path: "/x", value: "two" }] };
		await render(surface);
		expect(probeText("b")).toBe("two");
		expect(probeText("a")).toBe("static");
	});

	test("template rows follow their data and keep per-row scope", async () => {
		let surface = surfaceOf(
			[
				column("root", {
					template: { dataPath: "/items", templateComponentId: "item" },
				}),
				probe("item", { path: "$item.name" }),
			],
			{
				dataModel: [{ path: "/items", value: [{ name: "n1" }, { name: "n2" }] }],
			},
		);
		await mount(surface);
		const rows = () =>
			Array.from(host?.querySelectorAll('[data-probe="item"]') ?? []).map(
				(el) => el.textContent,
			);
		expect(rows()).toEqual(["n1", "n2"]);

		surface = {
			...surface,
			dataModel: [
				{
					path: "/items",
					value: [{ name: "n1" }, { name: "x" }, { name: "n3" }],
				},
			],
		};
		await render(surface);
		expect(rows()).toEqual(["n1", "x", "n3"]);
	});

	test("switching to another surface re-renders everything against it", async () => {
		await mount(
			surfaceOf([column("root", { explicitList: ["a"] }), probe("a", "one")]),
		);
		await render({
			id: "s2",
			rootComponentId: "top",
			components: {
				top: column("top", { explicitList: ["a"] }),
				a: probe("a", "other"),
			},
		});
		expect(probeText("a")).toBe("other");
		expect(
			host
				?.querySelector('[data-probe="a"]')
				?.getAttribute("data-a2ui-element-ref"),
		).toBe("s2/a");
	});
});
