import { afterAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { IUseInterfaceProps } from "./interfaces";

const handleServerMessage = mock((_message: unknown) => {});
// bun keeps a module mock for every later file in the process, so each one is put back in afterAll.
const actual = {
	locales: { ...(await import("@flow-like/locales")) },
	renderer: { ...(await import("../a2ui/A2UIRenderer")) },
	surfaceManager: { ...(await import("../a2ui/SurfaceManager")) },
	idbStorage: { ...(await import("../../lib/idb-storage")) },
};
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
mock.module("../a2ui/A2UIRenderer", () => ({
	...actual.renderer,
	A2UIRenderer: () => null,
}));
mock.module("../a2ui/SurfaceManager", () => ({
	...actual.surfaceManager,
	useSurfaceManager: () => ({
		surfaces: new Map(),
		handleServerMessage,
		getAllSurfaces: () => [],
	}),
}));
mock.module("../../lib/idb-storage", () => ({
	...actual.idbStorage,
	appGlobalState: {
		getAll: async () => ({}),
		set: async () => {},
	},
	pageLocalState: {
		getAll: async () => ({}),
		set: async () => {},
		clearPage: async () => {},
	},
}));
afterAll(() => {
	mock.restore();
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("../a2ui/A2UIRenderer", () => actual.renderer);
	mock.module("../a2ui/SurfaceManager", () => actual.surfaceManager);
	mock.module("../../lib/idb-storage", () => actual.idbStorage);
});

describe("A2UIInterface streamed state", () => {
	test("shares streamed state with the app and switches scope when the app changes", async () => {
		const window = new Window({ url: "https://local/use" });
		const streams: TestEventSource[] = [];
		class TestEventSource {
			onmessage?: (event: { data: string }) => void;
			onerror?: () => void;
			closed = false;
			constructor(_url: string) {
				streams.push(this);
			}
			close() {
				this.closed = true;
			}
			emit(message: unknown) {
				this.onmessage?.({ data: JSON.stringify(message) });
			}
		}
		const globals = {
			window,
			document: window.document,
			HTMLElement: window.HTMLElement,
			Node: window.Node,
			navigator: window.navigator,
			EventSource: TestEventSource,
			IS_REACT_ACT_ENVIRONMENT: true,
		};
		const previous = Object.fromEntries(
			Object.keys(globals).map((key) => [
				key,
				Object.getOwnPropertyDescriptor(globalThis, key),
			]),
		);
		Object.assign(globalThis, globals);
		const host = window.document.createElement("div");
		window.document.body.append(host);
		const root = createRoot(host as unknown as HTMLElement);
		try {
			const { A2UIInterface } = await import("./a2ui-interface");
			const { getFrontendStateStore } = await import("../a2ui/frontend-state");
			const event = { id: "event" } as IUseInterfaceProps["event"];
			const render = (appId: string) =>
				act(() => {
					root.render(
						<A2UIInterface
							appId={appId}
							event={event}
							config={{ streamUrl: "/stream" }}
						/>,
					);
				});
			await render("stream-app-a");
			streams[0].emit({ type: "setGlobalState", key: "theme", value: "dark" });
			streams[0].emit({
				type: "setPageState",
				page_id: "page-a",
				key: "selected",
				value: 7,
			});
			const firstStore = getFrontendStateStore("stream-app-a");
			expect(firstStore.getSnapshot().globalState).toEqual({ theme: "dark" });
			expect(firstStore.getSnapshot().pageStates["page-a"]).toEqual({
				selected: 7,
			});
			expect(handleServerMessage).not.toHaveBeenCalled();

			const surfaceMessage = { type: "deleteSurface", surfaceId: "page-a" };
			streams[0].emit(surfaceMessage);
			expect(handleServerMessage).toHaveBeenCalledWith(surfaceMessage);
			streams[0].emit({ type: "clearPageState", page_id: "page-a" });
			expect(firstStore.getSnapshot().pageStates["page-a"]).toEqual({});

			await render("stream-app-b");
			expect(streams[0].closed).toBe(true);
			streams[1].emit({ type: "setGlobalState", key: "theme", value: "light" });
			expect(
				getFrontendStateStore("stream-app-b").getSnapshot().globalState,
			).toEqual({
				theme: "light",
			});
			expect(firstStore.getSnapshot().globalState).toEqual({ theme: "dark" });
		} finally {
			await act(() => root.unmount());
			for (const [key, descriptor] of Object.entries(previous)) {
				if (descriptor) Object.defineProperty(globalThis, key, descriptor);
				else Reflect.deleteProperty(globalThis, key);
			}
		}
	});
});
