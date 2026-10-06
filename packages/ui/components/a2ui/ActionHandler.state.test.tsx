import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import {
	openRuntimeNamespace,
	runtimeDomEventName,
} from "../../lib/service-runtime/session-scope";
import type { Action } from "./types";

let root: Root | undefined;
const cleanup: (() => void)[] = [];

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	for (const restore of cleanup.reverse()) restore();
	cleanup.length = 0;
});

describe("workflow state round trip", () => {
	test("uses current page state and shares global writes with a mounted sibling", async () => {
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

		const [
			{ ActionProvider, useActionContext, useExecuteAction },
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
			spyOn(appGlobalState, "getAll").mockResolvedValue({ theme: "light" }),
			spyOn(pageLocalState, "getAll").mockImplementation(async (_app, page) =>
				page === "page-a" ? { count: 1 } : {},
			),
			spyOn(appGlobalState, "set").mockResolvedValue(),
			spyOn(pageLocalState, "set").mockResolvedValue(),
			spyOn(pageLocalState, "clearPage").mockResolvedValue(),
			spyOn(uiState.uiElementValues, "getAll").mockResolvedValue({}),
			spyOn(uiState, "pruneElementValues").mockResolvedValue(),
		];
		cleanup.push(() => {
			for (const spy of spies) spy.mockRestore();
		});

		const payloads: Record<string, unknown>[] = [];
		const previousBackend = useBackendStore.getState().backend;
		cleanup.push(() => useBackendStore.setState({ backend: previousBackend }));
		useBackendStore.getState().setBackend({
			boardState: {},
			eventState: {
				executeEvent: async (
					_appId: string,
					_eventId: string,
					run: { id: string; payload: Record<string, unknown> },
					_stream: boolean,
					onStarted: (id: string) => void,
					onEvents: (events: unknown[]) => void,
				) => {
					payloads.push(run.payload);
					onStarted("run-state");
					if (run.id === "clear-input") {
						onEvents([
							{
								event_type: "a2ui",
								payload: {
									type: "clearFileInput",
									surfaceId: "page-a",
									componentId: "upload",
								},
							},
						]);
					} else if (run.id === "write") {
						onEvents([
							{
								event_type: "a2ui",
								payload: {
									type: "setPageState",
									page_id: "page-a",
									key: "count",
									value: 2,
								},
							},
							{
								event_type: "a2ui",
								payload: {
									type: "setGlobalState",
									key: "theme",
									value: "dark",
								},
							},
						]);
					} else if (run.id === "clear") {
						onEvents([
							{
								event_type: "a2ui",
								payload: { type: "clearPageState", page_id: "page-a" },
							},
						]);
					}
				},
			},
		} as never);

		const controls: Record<string, ReturnType<typeof useExecuteAction>> = {};
		const contexts: Record<string, ReturnType<typeof useActionContext>> = {};
		function Probe({ pageId }: { pageId: string }) {
			controls[pageId] = useExecuteAction();
			contexts[pageId] = useActionContext();
			return null;
		}
		const appId = `state-roundtrip-${crypto.randomUUID()}`;
		const runtimeAppId = `device-runtime:${crypto.randomUUID()}`;
		cleanup.push(openRuntimeNamespace(runtimeAppId));
		const host = window.document.createElement("div");
		window.document.body.appendChild(host);
		root = createRoot(host as unknown as HTMLElement);
		await act(async () => {
			root?.render(
				<AppRouterContext.Provider value={{} as never}>
					<PathnameContext.Provider value="/use">
						{["page-a", "page-b", "runtime"].map((pageId) => (
							<ActionProvider
								key={pageId}
								appId={pageId === "runtime" ? runtimeAppId : appId}
								surfaceId={pageId}
								eventId="event-1"
								governedPage
								isPreviewMode
								components={{}}
							>
								<Probe pageId={pageId} />
							</ActionProvider>
						))}
					</PathnameContext.Provider>
				</AppRouterContext.Provider>,
			);
		});

		const action = (id: string): Action => ({
			name: "workflow_event",
			context: {},
			pageAction: { actionId: id, manifestRevision: "revision-1" },
		});
		// Keep the original callbacks while React batches renders between actions.
		const runA = controls["page-a"].executeAction;
		const runB = controls["page-b"].executeAction;
		await act(async () => {
			await runA(action("write"));
			await runA(action("read"));
			await runB(action("read"));
			await runA(action("clear"));
			await runA(action("read"));
		});

		expect(payloads).toHaveLength(5);
		expect(payloads[0]._page_id).toBe("page-a");
		expect(payloads[0]._page_state).toEqual({ count: 1 });
		expect(payloads[0]._global_state).toEqual({ theme: "light" });
		expect(payloads[1]._page_state).toEqual({ count: 2 });
		expect(payloads[1]._global_state).toEqual({ theme: "dark" });
		expect(payloads[2]._page_id).toBe("page-b");
		expect(payloads[2]._page_state).toEqual({});
		expect(payloads[2]._global_state).toEqual({ theme: "dark" });
		expect(payloads[4]._page_state).toEqual({});
		expect(contexts["page-b"].globalState).toEqual({ theme: "dark" });

		let hostClears = 0;
		let runtimeClears = 0;
		let otherRuntimeClears = 0;
		window.addEventListener("a2ui:clearFileInput", () => hostClears++);
		window.addEventListener(
			runtimeDomEventName("a2ui:clearFileInput", runtimeAppId),
			() => runtimeClears++,
		);
		window.addEventListener(
			runtimeDomEventName("a2ui:clearFileInput", "device-runtime:another-view"),
			() => otherRuntimeClears++,
		);
		await act(async () => {
			await controls.runtime.executeAction(action("clear-input"));
		});
		expect([hostClears, runtimeClears, otherRuntimeClears]).toEqual([0, 1, 0]);
		await act(async () => {
			await runA(action("clear-input"));
		});
		expect([hostClears, runtimeClears, otherRuntimeClears]).toEqual([1, 1, 0]);
	});
});
