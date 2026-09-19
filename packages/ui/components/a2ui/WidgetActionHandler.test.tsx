import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { appGlobalState, pageLocalState } from "../../lib/idb-storage";
import { useBackendStore } from "../../state/backend-state";
import {
	WidgetActionProvider,
	readWidgetActionBinding,
	useWidgetAction,
} from "./WidgetActionHandler";
import type { ActionBinding, WidgetAction } from "./types";

const INHERITED_ACTION_IDS = ["constructor", "toString", "__proto__"];

const commandBinding: ActionBinding = {
	command: { commandName: "refresh", args: {} },
};

// JSON.parse keeps `__proto__` as an own key, as a persisted page would.
const ownPrototypeNamedBindings = (): Record<string, ActionBinding> => {
	const binding = JSON.stringify(commandBinding);
	return JSON.parse(`{"constructor":${binding},"__proto__":${binding}}`);
};

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
	for (const cleanup of cleanups.splice(0)) await cleanup();
});

describe("readWidgetActionBinding", () => {
	test("inherited binding names resolve to no binding", () => {
		for (const actionId of INHERITED_ACTION_IDS) {
			expect(readWidgetActionBinding({}, actionId)).toBeNull();
			expect(
				readWidgetActionBinding({ submit: commandBinding }, actionId),
			).toBeNull();
		}
		expect(readWidgetActionBinding(undefined, "constructor")).toBeNull();
	});

	test("still reads own bindings named like prototype members", () => {
		const bindings = ownPrototypeNamedBindings();
		expect(readWidgetActionBinding(bindings, "constructor")).toEqual(
			commandBinding,
		);
		expect(readWidgetActionBinding(bindings, "__proto__")).toEqual(
			commandBinding,
		);
		expect(readWidgetActionBinding(bindings, "toString")).toBeNull();
	});
});

async function renderWidgetAction(
	actionId: string,
	actionBindings: Record<string, ActionBinding>,
	contextSchema: WidgetAction["contextSchema"] = [],
) {
	const window = new Window({ url: "https://local/use" });
	Object.assign(globalThis, {
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		window,
		IS_REACT_ACT_ENVIRONMENT: true,
	});

	const widgetAction: WidgetAction = {
		id: actionId,
		label: actionId,
		contextSchema,
	};
	const probe: { current: ReturnType<typeof useWidgetAction> | null } = {
		current: null,
	};
	function Probe() {
		probe.current = useWidgetAction(actionId);
		return null;
	}

	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	const root = createRoot(host as unknown as HTMLElement);
	cleanups.push(async () => {
		await act(() => root.unmount());
		await window.happyDOM.abort();
	});

	await act(() => {
		root.render(
			createElement(
				WidgetActionProvider,
				{
					instance: {
						instanceId: "inst-1",
						widgetId: "widget-1",
						customizationValues: {},
						actionBindings,
					},
					widgetActions: [widgetAction],
					appId: "app-1",
					surfaceId: "page-1",
				} as never,
				createElement(Probe),
			),
		);
	});

	return { probe, window };
}

/**
 * Record `a2ui:command` dispatches. Bun's global CustomEvent is not a happy-dom
 * Event (happy-dom would reject it), and happy-dom dispatches its own window
 * events, so only command events are kept.
 */
function captureCommands(window: Window): string[] {
	const commands: string[] = [];
	spyOn(window, "dispatchEvent").mockImplementation((event) => {
		if (event.type !== "a2ui:command") return true;
		const { detail } = event as unknown as CustomEvent<{ command: string }>;
		commands.push(detail.command);
		return true;
	});
	return commands;
}

describe("WidgetActionProvider bindings", () => {
	for (const actionId of INHERITED_ACTION_IDS) {
		test(`an unbound "${actionId}" action is unbound and never runs`, async () => {
			const warn = spyOn(console, "warn").mockImplementation(() => {});
			const log = spyOn(console, "log").mockImplementation(() => {});
			try {
				const { probe, window } = await renderWidgetAction(actionId, {});
				const commands = captureCommands(window);

				expect(probe.current?.binding).toBeNull();
				expect(probe.current?.isBound).toBe(false);

				await act(() => probe.current?.trigger());

				expect(warn).toHaveBeenCalledWith(
					`[WidgetAction] No binding found for action: ${actionId}`,
				);
				expect(log).not.toHaveBeenCalled();
				expect(commands).toEqual([]);
			} finally {
				warn.mockRestore();
				log.mockRestore();
			}
		});
	}

	test("an own binding named like a prototype member still runs", async () => {
		const log = spyOn(console, "log").mockImplementation(() => {});
		try {
			const { probe, window } = await renderWidgetAction(
				"constructor",
				ownPrototypeNamedBindings(),
			);
			const commands = captureCommands(window);

			expect(probe.current?.binding).toEqual(commandBinding);
			expect(probe.current?.isBound).toBe(true);

			await act(() => probe.current?.trigger());

			expect(commands).toEqual(["refresh"]);
		} finally {
			log.mockRestore();
		}
	});
});

describe("WidgetActionProvider input mappings", () => {
	test("inherited mapping names fall back to the trigger context", async () => {
		const runs: Record<string, unknown>[] = [];
		const previousBackend = useBackendStore.getState().backend;
		useBackendStore.getState().setBackend({
			boardState: {
				executeBoard: async (
					_appId: string,
					_boardId: string,
					run: { payload: Record<string, unknown> },
				) => {
					runs.push(run.payload);
				},
			},
			eventState: {},
		} as never);
		const spies = [
			spyOn(appGlobalState, "getAll").mockResolvedValue({}),
			spyOn(pageLocalState, "getAll").mockResolvedValue({}),
			spyOn(console, "log").mockImplementation(() => {}),
		];
		try {
			// An own `constructor` mapping (as JSON.parse keeps it) still applies;
			// names only reachable through the prototype chain are no mapping.
			const inputMappings = Object.setPrototypeOf(
				JSON.parse('{"constructor":{"literalString":"mapped"}}'),
				{
					toString: { literalString: "inherited" },
					region: { literalString: "inherited" },
				},
			);
			const field = (name: string) => ({
				name,
				label: name,
				fieldType: "String",
			});
			const { probe } = await renderWidgetAction(
				"submit",
				{ submit: { workflow: { flowId: "board-1", inputMappings } } },
				[field("constructor"), field("toString"), field("region")],
			);

			await act(() =>
				probe.current?.trigger({
					toString: "from context",
					region: "from context",
				}),
			);

			expect(runs).toHaveLength(1);
			expect(runs[0].constructor).toBe("mapped");
			expect(runs[0].toString).toBe("from context");
			expect(runs[0].region).toBe("from context");
		} finally {
			for (const spy of spies) spy.mockRestore();
			useBackendStore.setState({ backend: previousBackend });
		}
	});
});
