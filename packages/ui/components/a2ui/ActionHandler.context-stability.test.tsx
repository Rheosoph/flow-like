import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import { act, memo } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { SurfaceComponent } from "./types";

let root: Root | undefined;
const cleanup: (() => void)[] = [];

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	for (const restore of cleanup.reverse()) restore();
	cleanup.length = 0;
});

const text = (id: string, value: string): SurfaceComponent => ({
	id,
	component: { type: "text", content: { literalString: value } },
});

describe("ActionProvider context stability", () => {
	test("a surface update leaves consumers alone while getComponents stays fresh", async () => {
		const window = new Window({ url: "https://local/use" });
		const globals = {
			window,
			document: window.document,
			navigator: window.navigator,
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

		const [
			{ ActionProvider, useAgentActionAccess, useExecuteAction, useOnAction },
			{ useBackendStore },
			{ appGlobalState, pageLocalState },
			uiState,
			{ AppRouterContext },
			{ PathnameContext },
		] = await Promise.all([
			import("./ActionHandler"),
			import("../../state/backend-state"),
			import("../../lib/idb-storage"),
			import("../../db/ui-state-db"),
			import("next/dist/shared/lib/app-router-context.shared-runtime"),
			import("next/dist/shared/lib/hooks-client-context.shared-runtime"),
		]);
		const spies = [
			spyOn(appGlobalState, "getAll").mockResolvedValue({}),
			spyOn(pageLocalState, "getAll").mockResolvedValue({}),
			spyOn(appGlobalState, "set").mockResolvedValue(),
			spyOn(pageLocalState, "set").mockResolvedValue(),
			spyOn(pageLocalState, "clearPage").mockResolvedValue(),
			spyOn(uiState.uiElementValues, "getAll").mockResolvedValue({}),
			spyOn(uiState, "pruneElementValues").mockResolvedValue(),
		];
		cleanup.push(() => {
			for (const spy of spies) spy.mockRestore();
		});

		const previousBackend = useBackendStore.getState().backend;
		cleanup.push(() => useBackendStore.setState({ backend: previousBackend }));
		useBackendStore.getState().setBackend({
			boardState: {},
			eventState: {},
		} as never);

		let consumerRenders = 0;
		const seen: {
			onAction?: unknown;
			executeAction?: unknown;
			getComponents?: () => Record<string, SurfaceComponent> | undefined;
		} = {};
		// Memoized like the real component nodes: only a context change can
		// re-render it.
		const Consumer = memo(function Consumer() {
			consumerRenders += 1;
			seen.onAction = useOnAction();
			seen.executeAction = useExecuteAction().executeAction;
			seen.getComponents = useAgentActionAccess().getComponents;
			return null;
		});

		const host = window.document.createElement("div");
		window.document.body.appendChild(host);
		root = createRoot(host as unknown as HTMLElement);
		const onAction = () => {};
		// Host contexts must be stable themselves, or useRouter() consumers re-render.
		const router = {} as never;
		const mount = (components: Record<string, SurfaceComponent>) =>
			act(async () => {
				root?.render(
					<AppRouterContext.Provider value={router}>
						<PathnameContext.Provider value="/use">
							<ActionProvider
								appId="stability-app"
								surfaceId="page"
								eventId="event-1"
								isPreviewMode
								onAction={onAction}
								components={components}
							>
								<Consumer />
							</ActionProvider>
						</PathnameContext.Provider>
					</AppRouterContext.Provider>,
				);
			});

		const v1 = { a: text("a", "one") };
		await mount(v1);
		// Async storage restoration may settle in a later render; measure after it.
		await mount(v1);
		const settledRenders = consumerRenders;
		const firstOnAction = seen.onAction;
		const firstExecute = seen.executeAction;
		expect(seen.getComponents?.()).toBe(v1);

		const v2 = { a: text("a", "two") };
		await mount(v2);

		expect(consumerRenders).toBe(settledRenders);
		expect(seen.onAction).toBe(firstOnAction);
		expect(seen.executeAction).toBe(firstExecute);
		expect(seen.getComponents?.()).toBe(v2);
	});
});
