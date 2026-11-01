import { afterEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement, useState } from "react";
import type { IIntercomEvent } from "../lib/schema/events/intercom-event";
import type { IEvent } from "../lib/schema/flow/event";
import type { ILogMetadata } from "../lib/schema/flow/log-metadata";
import type { IEventState } from "../state/backend-state/event-state";
import { useQuickAction } from "./use-quick-action";

const window = new Window({ url: "https://localhost" });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	IS_REACT_ACT_ENVIRONMENT: true,
});

const { createRoot } = await import("react-dom/client");
const roots: ReturnType<typeof createRoot>[] = [];

afterEach(() => {
	act(() => {
		for (const root of roots.splice(0)) root.unmount();
	});
	window.document.body.replaceChildren();
});

const event = { id: "quick-action", node_id: "start" };
const otherEvent = { id: "other-action", node_id: "other-start" };

function intercom(eventType: string, payload: unknown): IIntercomEvent {
	return {
		event_id: "message",
		event_type: eventType,
		payload,
		timestamp: { secs_since_epoch: 1, nanos_since_epoch: 0 },
	};
}

function metadata(overrides: Partial<ILogMetadata> = {}): ILogMetadata {
	return {
		app_id: "app",
		board_id: "board",
		end: 2,
		event_id: event.id,
		log_level: 1,
		node_id: event.node_id,
		payload: [],
		run_id: "run",
		start: 1,
		version: "1.0.0",
		...overrides,
	};
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((done, fail) => {
		resolve = done;
		reject = fail;
	});
	return { promise, resolve, reject };
}

function mount(
	executeEvent: IEventState["executeEvent"],
	initialAppId = "app",
) {
	let quickAction!: ReturnType<typeof useQuickAction>;
	let setApp!: (appId: string) => void;
	const onError = mock((_error: unknown) => {});

	function Harness() {
		const [appId, setAppId] = useState(initialAppId);
		setApp = setAppId;
		quickAction = useQuickAction({
			appId,
			executeEvent,
			streamState: false,
			onError,
		});
		return createElement("button", {
			type: "button",
			disabled: quickAction.isRunning(event.id),
			onClick: () => void quickAction.execute(event),
		});
	}

	const container = window.document.createElement("div");
	window.document.body.appendChild(container);
	const root = createRoot(container as unknown as HTMLElement);
	roots.push(root);
	act(() => root.render(createElement(Harness)));
	return {
		get action() {
			return quickAction;
		},
		get button() {
			const button = container.querySelector("button");
			if (!button) throw new Error("Quick action button missing");
			return button;
		},
		onError,
		switchApp(appId: string) {
			act(() => setApp(appId));
		},
	};
}

describe("manual quick actions", () => {
	const versions: [string, IEvent["board_version"]][] = [
		["Latest", null],
		["fixed version", [1, 2, 3]],
	];
	test.each(versions)(
		"a remote %s run can complete without local log metadata",
		async (_label, boardVersion) => {
			const executeEvent = mock<IEventState["executeEvent"]>(
				async (...args) => {
					args[5]?.([intercom("completed", { status: "completed" })]);
					return undefined;
				},
			);
			const view = mount(executeEvent);
			const configuredEvent: Pick<IEvent, "id" | "node_id" | "board_version"> =
				{
					...event,
					board_version: boardVersion,
				};
			await act(async () => view.action.execute(configuredEvent));

			expect(view.onError).not.toHaveBeenCalled();
			expect(view.button.disabled).toBe(false);
			expect(executeEvent).toHaveBeenCalledTimes(1);
			expect(executeEvent.mock.calls[0].slice(0, 4)).toEqual([
				"app",
				event.id,
				{ id: event.node_id },
				false,
			]);
		},
	);

	test("an invocation resolving without metadata or events is not a failure", async () => {
		const view = mount(async () => undefined);
		await act(async () => view.action.execute(event));
		expect(view.onError).not.toHaveBeenCalled();
	});

	test("local success metadata is accepted", async () => {
		const view = mount(async () => metadata());
		await act(async () => view.action.execute(event));
		expect(view.onError).not.toHaveBeenCalled();
	});

	test("local failure metadata reports the workflow reason", async () => {
		const view = mount(async () =>
			metadata({ log_level: 3, error_message: "Cannot write invoice" }),
		);
		await act(async () => view.action.execute(event));
		expect(view.onError).toHaveBeenCalledTimes(1);
		expect(view.onError.mock.calls[0][0]).toMatchObject({
			message: "Cannot write invoice",
		});
	});

	test.each([
		["error", { message: "Server execution failed" }],
		[
			"completed",
			{ status: "failed", error_message: "Server execution failed" },
		],
	])(
		"a streamed %s reports failure even when the transport resolves",
		async (type, payload) => {
			const view = mount(async (...args) => {
				args[5]?.([intercom(type, payload)]);
				return undefined;
			});
			await act(async () => view.action.execute(event));
			expect(view.onError).toHaveBeenCalledTimes(1);
			expect(view.onError.mock.calls[0][0]).toMatchObject({
				message: "Server execution failed",
			});
			expect(view.button.disabled).toBe(false);
		},
	);

	test("a successful completion supersedes handled node errors", async () => {
		const view = mount(async (...args) => {
			args[5]?.([intercom("error", { message: "Handled failure" })]);
			args[5]?.([intercom("completed", { status: "success" })]);
			return undefined;
		});
		await act(async () => view.action.execute(event));
		expect(view.onError).not.toHaveBeenCalled();
	});

	test("a rejected invocation reports once and releases the button without retrying", async () => {
		const pending = deferred<ILogMetadata | undefined>();
		const executeEvent = mock(() => pending.promise);
		const view = mount(executeEvent);
		let execution!: Promise<void>;
		act(() => {
			execution = view.action.execute(event);
		});
		expect(view.button.disabled).toBe(true);
		const error = new Error("Permission denied");
		await act(async () => {
			pending.reject(error);
			await execution;
		});
		expect(view.onError).toHaveBeenCalledTimes(1);
		expect(view.onError).toHaveBeenCalledWith(error);
		expect(view.button.disabled).toBe(false);
		expect(executeEvent).toHaveBeenCalledTimes(1);
	});

	test("same-tick double clicks dispatch once and a later explicit click can run again", async () => {
		const pending = deferred<ILogMetadata | undefined>();
		const executeEvent = mock(() => pending.promise);
		const view = mount(executeEvent);
		let first!: Promise<void>;
		let second!: Promise<void>;
		act(() => {
			first = view.action.execute(event);
			second = view.action.execute(event);
		});
		expect(executeEvent).toHaveBeenCalledTimes(1);
		expect(view.button.disabled).toBe(true);
		act(() => view.button.click());
		expect(executeEvent).toHaveBeenCalledTimes(1);
		await act(async () => {
			pending.resolve(undefined);
			await Promise.all([first, second]);
		});
		expect(view.button.disabled).toBe(false);
		await act(async () => view.action.execute(event));
		expect(executeEvent).toHaveBeenCalledTimes(2);
		expect(view.onError).not.toHaveBeenCalled();
	});

	test("different events can run independently", async () => {
		const first = deferred<ILogMetadata | undefined>();
		const second = deferred<ILogMetadata | undefined>();
		const executeEvent = mock<IEventState["executeEvent"]>(async (_app, id) =>
			id === event.id ? first.promise : second.promise,
		);
		const view = mount(executeEvent);
		let execution!: Promise<void>;
		let otherExecution!: Promise<void>;
		act(() => {
			execution = view.action.execute(event);
			otherExecution = view.action.execute(otherEvent);
		});
		expect(executeEvent).toHaveBeenCalledTimes(2);
		expect(view.action.isRunning(event.id)).toBe(true);
		expect(view.action.isRunning(otherEvent.id)).toBe(true);
		await act(async () => {
			first.resolve(undefined);
			await execution;
		});
		expect(view.action.isRunning(event.id)).toBe(false);
		expect(view.action.isRunning(otherEvent.id)).toBe(true);
		await act(async () => {
			second.resolve(undefined);
			await otherExecution;
		});
	});

	test("app navigation scopes pending actions and preserves the guard when returning", async () => {
		const first = deferred<ILogMetadata | undefined>();
		const second = deferred<ILogMetadata | undefined>();
		const executeEvent = mock<IEventState["executeEvent"]>(async (appId) =>
			appId === "app" ? first.promise : second.promise,
		);
		const view = mount(executeEvent);
		let execution!: Promise<void>;
		let otherExecution!: Promise<void>;
		act(() => {
			execution = view.action.execute(event);
		});
		view.switchApp("other-app");
		expect(view.button.disabled).toBe(false);
		act(() => {
			otherExecution = view.action.execute(event);
		});
		expect(executeEvent).toHaveBeenCalledTimes(2);
		view.switchApp("app");
		expect(view.button.disabled).toBe(true);
		await act(async () => view.action.execute(event));
		expect(executeEvent).toHaveBeenCalledTimes(2);
		await act(async () => {
			first.resolve(undefined);
			await execution;
		});
		expect(view.button.disabled).toBe(false);
		view.switchApp("other-app");
		expect(view.button.disabled).toBe(true);
		await act(async () => {
			second.resolve(undefined);
			await otherExecution;
		});
		expect(view.button.disabled).toBe(false);
	});

	test("no app means no dispatch", async () => {
		const executeEvent = mock(async () => undefined);
		const view = mount(executeEvent, "");
		await act(async () => view.action.execute(event));
		expect(executeEvent).not.toHaveBeenCalled();
		expect(view.onError).not.toHaveBeenCalled();
	});
});
