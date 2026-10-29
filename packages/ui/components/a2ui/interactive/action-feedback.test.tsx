import {
	afterEach,
	describe,
	expect,
	setDefaultTimeout,
	spyOn,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { flushSync } from "react-dom";
import { type Root, createRoot } from "react-dom/client";
import type { Action, ButtonComponent, LinkComponent } from "../types";

setDefaultTimeout(60_000);

let root: Root | undefined;
const cleanup: (() => void | Promise<void>)[] = [];

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	for (const restore of cleanup.reverse()) await restore();
	cleanup.length = 0;
});

const workflowAction = (actionId: string): Action => ({
	name: "workflow_event",
	context: {},
	pageAction: { actionId, manifestRevision: "revision-1" },
});

async function setup(
	kind: "button" | "link",
	actions: Action[] = [workflowAction("load-details")],
) {
	const window = new Window({ url: "https://local/use" });
	Object.assign(window, { SyntaxError, TypeError });
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
	cleanup.push(async () => {
		await window.happyDOM.close();
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	});

	const [
		{ ActionProvider },
		{ DataProvider },
		{ A2UIButton },
		{ A2UILink },
		{ useBackendStore },
		{ appGlobalState, pageLocalState },
		uiState,
		{ AppRouterContext },
		{ PathnameContext },
		{ toast },
	] = await Promise.all([
		import("../ActionHandler"),
		import("../DataContext"),
		import("./Button"),
		import("./Link"),
		import("../../../state/backend-state"),
		import("../../../lib/idb-storage"),
		import("../../../db/ui-state-db"),
		import("next/dist/shared/lib/app-router-context.shared-runtime"),
		import("next/dist/shared/lib/hooks-client-context.shared-runtime"),
		import("sonner"),
	]);
	const errorToast = spyOn(toast, "error").mockImplementation(() => "");
	const spies = [
		spyOn(appGlobalState, "getAll").mockResolvedValue({}),
		spyOn(pageLocalState, "getAll").mockResolvedValue({}),
		spyOn(uiState.uiElementValues, "getAll").mockResolvedValue({}),
		spyOn(uiState, "pruneElementValues").mockResolvedValue(),
		spyOn(console, "log").mockImplementation(() => {}),
		spyOn(console, "debug").mockImplementation(() => {}),
		spyOn(console, "warn").mockImplementation(() => {}),
		spyOn(console, "error").mockImplementation(() => {}),
		spyOn(toast, "success").mockImplementation(() => ""),
		errorToast,
	];
	cleanup.push(() => {
		for (const spy of spies) spy.mockRestore();
	});

	const runs: {
		id: string;
		resolve: () => void;
		reject: (reason: Error) => void;
	}[] = [];
	const startRun = (id: string) =>
		new Promise<void>((resolve, reject) => runs.push({ id, resolve, reject }));
	const previousBackend = useBackendStore.getState().backend;
	cleanup.push(() => useBackendStore.setState({ backend: previousBackend }));
	useBackendStore.getState().setBackend({
		boardState: {},
		eventState: {
			executeEvent: (
				_appId: string,
				_eventId: string,
				payload: { id: string },
			) => startRun(payload.id),
			upsertEventFeedback: () => startRun("feedback"),
		},
	} as never);

	const label = { literalString: "Details" };
	const component: ButtonComponent | LinkComponent =
		kind === "button"
			? { id: "details", type: kind, label, eventHandlers: { click: actions } }
			: {
					id: "details",
					type: kind,
					label,
					href: { literalString: "" },
					eventHandlers: { navigate: actions },
				};
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);
	await act(async () => {
		const props = {
			componentId: "details",
			surfaceId: `feedback-${kind}`,
			renderChild: () => null,
		};
		root?.render(
			<AppRouterContext.Provider value={{} as never}>
				<PathnameContext.Provider value="/use">
					<ActionProvider
						appId={`feedback-${kind}`}
						surfaceId={`feedback-${kind}`}
						eventId="page-event"
						governedPage
						isPreviewMode
						components={{ details: { id: "details", component } }}
					>
						<DataProvider initialData={[]}>
							{kind === "button" ? (
								<A2UIButton
									{...props}
									component={component as ButtonComponent}
								/>
							) : (
								<A2UILink {...props} component={component as LinkComponent} />
							)}
						</DataProvider>
					</ActionProvider>
				</PathnameContext.Provider>
			</AppRouterContext.Provider>,
		);
	});

	const button = host.querySelector("button");
	if (!button) throw new Error("Expected an action button");
	const pointerDown = () =>
		button.dispatchEvent(
			new window.PointerEvent("pointerdown", {
				bubbles: true,
				button: 0,
				isPrimary: true,
			}),
		);
	const click = () =>
		button.dispatchEvent(
			new window.MouseEvent("click", { bubbles: true, detail: 1 }),
		);
	const activate = () => {
		pointerDown();
		click();
	};

	return { button, runs, pointerDown, click, activate, window, errorToast };
}

describe("action control feedback", () => {
	for (const kind of ["button", "link"] as const) {
		test(`${kind} waits for a click, shows pending feedback, and ignores rapid repeats`, async () => {
			const { button, runs, pointerDown, click, activate, window } =
				await setup(kind);
			await act(() => {
				button.dispatchEvent(
					new window.PointerEvent("pointerover", { bubbles: true }),
				);
				pointerDown();
			});
			expect(runs).toHaveLength(0);
			expect(button.getAttribute("aria-busy")).toBe("false");

			await act(async () => {
				click();
				// Both activations arrive before React can commit disabled=true.
				activate();
			});
			expect(runs).toHaveLength(1);
			expect(button.disabled).toBe(true);
			expect(button.getAttribute("aria-busy")).toBe("true");
			expect(button.querySelector("svg.animate-spin")).not.toBeNull();
			expect(button.textContent).toBe("Details");

			await act(async () => runs[0].resolve());
			expect(button.disabled).toBe(false);
			expect(button.getAttribute("aria-busy")).toBe("false");
			await act(async () => activate());
			expect(runs).toHaveLength(2);
			await act(async () => runs[1].resolve());
		});

		test(`${kind} clears pending feedback after a failed run and permits retry`, async () => {
			const { button, runs, activate, errorToast } = await setup(kind);
			await act(async () => activate());
			expect(button.disabled).toBe(true);
			await act(async () => runs[0].reject(new Error("Request failed")));
			expect(errorToast).toHaveBeenCalledTimes(1);
			expect(button.disabled).toBe(false);
			expect(button.getAttribute("aria-busy")).toBe("false");
			expect(button.querySelector("svg.animate-spin")).toBeNull();
			await act(async () => activate());
			expect(runs).toHaveLength(2);
			await act(async () => runs[1].resolve());
		});

		test(`${kind} stays busy for the entire ordered event`, async () => {
			const { button, runs, activate, window } = await setup(kind, [
				workflowAction("first"),
				workflowAction("second"),
			]);
			await act(async () => activate());
			expect(runs.map((run) => run.id)).toEqual(["first"]);
			expect(button.getAttribute("aria-busy")).toBe("true");

			const busyChanges: (string | null)[] = [];
			const observer = new window.MutationObserver((records) => {
				for (const record of records) busyChanges.push(record.oldValue);
			});
			observer.observe(button, {
				attributes: true,
				attributeFilter: ["aria-busy"],
				attributeOldValue: true,
			});
			cleanup.push(() => observer.disconnect());

			await act(async () => {
				runs[0].resolve();
				// Commit at each async boundary to catch a briefly idle control.
				for (let turn = 0; turn < 20 && runs.length < 2; turn++) {
					await Promise.resolve();
					flushSync(() => {});
					expect(button.getAttribute("aria-busy")).toBe("true");
					activate();
				}
			});
			expect(runs.map((run) => run.id)).toEqual(["first", "second"]);
			expect(button.disabled).toBe(true);
			expect(button.getAttribute("aria-busy")).toBe("true");
			await act(async () => activate());
			expect(runs).toHaveLength(2);
			busyChanges.push(
				...observer.takeRecords().map((record) => record.oldValue),
			);
			expect(busyChanges).toEqual([]);

			await act(async () => runs[1].resolve());
			expect(button.disabled).toBe(false);
			expect(button.getAttribute("aria-busy")).toBe("false");
		});

		test(`${kind} handles a rejected action promise, stops its chain, and permits retry`, async () => {
			const { button, runs, activate, errorToast } = await setup(kind, [
				{ name: "submit_feedback", context: { rating: 5 } },
				workflowAction("after-feedback"),
			]);
			await act(async () => activate());
			expect(runs.map((run) => run.id)).toEqual(["feedback"]);
			await act(async () => runs[0].reject(new Error("Feedback failed")));
			expect(errorToast).toHaveBeenCalledTimes(1);
			expect(errorToast).toHaveBeenCalledWith("Something went wrong", {
				description: "Feedback failed",
			});
			expect(runs).toHaveLength(1);
			expect(button.disabled).toBe(false);
			expect(button.getAttribute("aria-busy")).toBe("false");

			await act(async () => activate());
			expect(runs.map((run) => run.id)).toEqual(["feedback", "feedback"]);
			await act(async () => runs[1].resolve());
			expect(runs.map((run) => run.id)).toEqual([
				"feedback",
				"feedback",
				"after-feedback",
			]);
			expect(button.disabled).toBe(true);
			await act(async () => runs[2].resolve());
			expect(button.disabled).toBe(false);
		});
	}
});
