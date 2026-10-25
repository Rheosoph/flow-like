import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { Root } from "react-dom/client";
import { applyElementUpdate } from "./apply-a2ui-message";
import type { SurfaceComponent } from "./types";

let root: Root | undefined;
const cleanup: (() => void)[] = [];

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	for (const restore of cleanup.reverse()) restore();
	cleanup.length = 0;
});

function installDom(): Window {
	const window = new Window({ url: "https://local/use" });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		CustomEvent: window.CustomEvent,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	cleanup.push(() => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	});
	return window;
}

type Components = Record<string, SurfaceComponent>;

const field = (id: string, value: string): SurfaceComponent =>
	({
		id,
		component: { type: "textField", value: { literalString: value } },
	}) as unknown as SurfaceComponent;

function setValue(
	components: Components,
	id: string,
	value: string,
): Components {
	return {
		...components,
		[id]: applyElementUpdate(components[id], { type: "setValue", value }),
	};
}

const sentValue = (elements: Record<string, unknown>, elementId: string) =>
	(elements[elementId] as { component: { value: unknown } }).component.value;

/** Mounts an ActionProvider on surface `page` whose storage holds `restored`. */
async function mountProvider(restored: Record<string, unknown>) {
	const window = installDom();
	const [
		{ createRoot },
		{ ActionProvider, useCollectEventElements, useOnAction },
		{ useBackendStore },
		uiState,
		{ AppRouterContext },
		{ PathnameContext },
	] = await Promise.all([
		import("react-dom/client"),
		import("./ActionHandler"),
		import("../../state/backend-state"),
		import("../../db/ui-state-db"),
		import("next/dist/shared/lib/app-router-context.shared-runtime"),
		import("next/dist/shared/lib/hooks-client-context.shared-runtime"),
	]);

	const records = Object.fromEntries(
		Object.entries(restored).map(([elementId, value]) => [
			elementId,
			{
				id: `restored-${elementId}`,
				appId: "app",
				elementId,
				value,
				updatedAt: Date.now(),
			},
		]),
	);
	const persisted: [string, unknown][] = [];
	const spies = [
		spyOn(uiState.uiElementValues, "getAll").mockResolvedValue(records),
		spyOn(uiState.uiElementValues, "set").mockImplementation(
			async (_appId, elementId, value) => {
				persisted.push([elementId, value]);
			},
		),
		spyOn(uiState, "pruneElementValues").mockResolvedValue(),
	];
	cleanup.push(() => {
		for (const spy of spies) spy.mockRestore();
	});

	const previousBackend = useBackendStore.getState().backend;
	cleanup.push(() => useBackendStore.setState({ backend: previousBackend }));
	useBackendStore
		.getState()
		.setBackend({ boardState: {}, eventState: {} } as never);

	const probe: {
		onAction?: ReturnType<typeof useOnAction>;
		collect?: ReturnType<typeof useCollectEventElements>;
	} = {};
	function Probe() {
		probe.onAction = useOnAction();
		probe.collect = useCollectEventElements();
		return null;
	}

	const appId = `flow-values-${crypto.randomUUID()}`;
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);

	return {
		persisted,
		render: (components: Components) =>
			act(async () => {
				root?.render(
					<AppRouterContext.Provider value={{} as never}>
						<PathnameContext.Provider value="/use">
							<ActionProvider
								appId={appId}
								surfaceId="page"
								isPreviewMode
								components={components}
							>
								<Probe />
							</ActionProvider>
						</PathnameContext.Provider>
					</AppRouterContext.Provider>,
				);
			}),
		collect: () => {
			if (!probe.collect) throw new Error("ActionProvider probe did not mount");
			return probe.collect();
		},
		settle: (ms = 0) =>
			act(() => new Promise((resolve) => setTimeout(resolve, ms))),
		edit: (componentId: string, value: string) =>
			act(() => {
				probe.onAction?.({
					type: "userAction",
					name: "change",
					surfaceId: "page",
					sourceComponentId: componentId,
					timestamp: Date.now(),
					context: { value },
				});
			}),
	};
}

describe("flow writes and the run payload", () => {
	test("an action after a flow write sends the value the screen shows", async () => {
		const page = await mountProvider({ "page/kcal": "208" });

		let surface: Components = { kcal: field("kcal", "210") };
		await page.render(surface);
		await page.settle();
		expect(sentValue(await page.collect(), "page/kcal")).toEqual({
			literalString: "210",
		});

		await page.edit("kcal", "211");
		surface = setValue(surface, "kcal", "405");
		await page.render(surface);
		expect(sentValue(await page.collect(), "page/kcal")).toEqual({
			literalString: "405",
		});

		await page.settle(350);
		expect(page.persisted).toEqual([["page/kcal", "405"]]);

		await page.edit("kcal", "410");
		await page.render({ ...surface });
		expect(sentValue(await page.collect(), "page/kcal")).toEqual({
			literalString: "410",
		});
	});

	test("a restored draft the field does not show never reaches the payload", async () => {
		const page = await mountProvider({
			"page/description": "Hähnchen",
			"page/kcal": "405",
			"page/later": "stale",
		});

		const surface: Components = {
			description: field("description", ""),
			kcal: field("kcal", "405"),
		};
		await page.render(surface);
		await page.settle();
		let sent = await page.collect();
		expect(sentValue(sent, "page/description")).toEqual({ literalString: "" });
		expect(sentValue(sent, "page/kcal")).toEqual({ literalString: "405" });
		await page.settle(350);
		expect(page.persisted).toEqual([]);

		await page.edit("description", "Pizza");
		await page.render({ ...surface, later: field("later", "") });
		sent = await page.collect();
		expect(sentValue(sent, "page/later")).toEqual({ literalString: "" });
		expect(sentValue(sent, "page/description")).toEqual({
			literalString: "Pizza",
		});
	});
});
