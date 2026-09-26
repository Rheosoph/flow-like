import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import type { RuntimeVariablesPromptProps } from "../components/flow/runtime-variables-prompt";
import type { WasmSandboxWarningDialogProps } from "../components/flow/wasm-sandbox-warning-dialog";
import type { ILogMetadata, IRunPayload } from "../lib";
import type { PageTrigger } from "../lib/schema/flow/page-trigger";
import type { IPrerunEventResponse } from "./backend-state/types";
import type { ExecutionServiceContextValue } from "./execution-service-context-value";
import type {
	RuntimeVariableValue,
	RuntimeVariablesContextValue,
} from "./runtime-variables-context";

const browser = new Window({ url: "https://flow-like.test/" });
const globals = {
	window: browser,
	document: browser.document,
	navigator: browser.navigator,
	localStorage: browser.localStorage,
	IS_REACT_ACT_ENVIRONMENT: true,
};
const originals = new Map(
	Object.keys(globals).map((key) => [
		key,
		Object.getOwnPropertyDescriptor(globalThis, key),
	]),
);
for (const [key, value] of Object.entries(globals))
	Object.defineProperty(globalThis, key, {
		configurable: true,
		writable: true,
		value,
	});

let runtimePrompt: RuntimeVariablesPromptProps | undefined;
let wasmPrompt: WasmSandboxWarningDialogProps | undefined;

const PROMPT_MODULE = "../components/flow/runtime-variables-prompt";
const WASM_MODULE = "../components/flow/wasm-sandbox-warning-dialog";
const PAYMENT_MODULE = "../components/payments/node-payment";
const BACKEND_MODULE = "./backend-state";
// bun keeps module mocks for later test files, so override only what this file needs.
const actualModules = new Map<string, Record<string, unknown>>([
	[PROMPT_MODULE, { ...(await import(PROMPT_MODULE)) }],
	[WASM_MODULE, { ...(await import(WASM_MODULE)) }],
	[PAYMENT_MODULE, { ...(await import(PAYMENT_MODULE)) }],
	[BACKEND_MODULE, { ...(await import(BACKEND_MODULE)) }],
]);
const overrides: Record<string, Record<string, unknown>> = {
	[PROMPT_MODULE]: {
		RuntimeVariablesPrompt: (props: RuntimeVariablesPromptProps) => {
			runtimePrompt = props;
			return null;
		},
	},
	[WASM_MODULE]: {
		WasmSandboxWarningDialog: (props: WasmSandboxWarningDialogProps) => {
			wasmPrompt = props;
			return null;
		},
	},
	[PAYMENT_MODULE]: { NodePaymentPrompt: () => null },
	[BACKEND_MODULE]: { useBackend: () => backend },
};
for (const [path, actual] of actualModules)
	mock.module(path, () => ({ ...actual, ...overrides[path] }));

const { createRoot } = await import("react-dom/client");
const providerModule = await import("./execution-service-provider");
const { ExecutionServiceProvider } = providerModule;
const { useExecutionService } = await import(
	"./execution-service-context-value"
);
const { RuntimeVariablesProvider } = await import(
	"./runtime-variables-context"
);

const CANCELLED = "Execution cancelled: runtime variables not configured";
const UNMOUNTED =
	"Execution cancelled: the execution service unmounted while the run waited for a prompt";
const LOAD: PageTrigger = {
	kind: "special",
	specialEvent: "load",
	manifestRevision: "revision",
};

const prerunByEvent = new Map<string, Partial<IPrerunEventResponse>>();
const executeEventRemote = mock(
	async (appId: string, eventId: string, payload: IRunPayload) =>
		({ app_id: appId, event_id: eventId, payload }) as unknown as ILogMetadata,
);
const backend = {
	boardState: {},
	eventState: {
		alwaysRemote: true,
		executeEvent: mock(async () => undefined),
		executeEventRemote,
		prerunEvent: mock(
			async (_appId: string, eventId: string): Promise<IPrerunEventResponse> =>
				({
					board_id: `board-${eventId}`,
					oauth_requirements: [],
					requires_local_execution: false,
					execution_mode: "Remote",
					can_execute_locally: false,
					runtime_variables: [],
					...prerunByEvent.get(eventId),
				}) as IPrerunEventResponse,
		),
	},
};

const stored = new Map<string, Map<string, RuntimeVariableValue>>();
let saveGate: Promise<void> | undefined;
const saveValues = mock(
	async (
		appId: string,
		_boardId: string,
		values: { variableId: string; value: number[] }[],
	) => {
		await saveGate;
		const app = stored.get(appId) ?? new Map<string, RuntimeVariableValue>();
		for (const { variableId, value } of values)
			app.set(variableId, { variableId, value });
		stored.set(appId, app);
	},
);
const store = {
	getValues: async (appId: string) => new Map(stored.get(appId)),
	hasAllValues: async (appId: string, ids: string[]) =>
		ids.every((id) => stored.get(appId)?.has(id)),
	saveValues,
} as unknown as RuntimeVariablesContextValue;

let root: ReturnType<typeof createRoot>;
let mounted = false;
let service: ExecutionServiceContextValue;

function Probe() {
	service = useExecutionService();
	return null;
}

function needsInput(eventId: string, variableId: string) {
	prerunByEvent.set(eventId, {
		runtime_variables: [
			{
				id: variableId,
				name: variableId,
				data_type: "String",
				value_type: "Normal",
				secret: false,
			},
		],
	});
}

function needsWasmConsent(eventId: string, packageId: string) {
	prerunByEvent.set(eventId, {
		has_wasm_nodes: true,
		wasm_package_ids: [packageId],
	});
}

function track(promise: Promise<ILogMetadata | undefined>) {
	const outcome: {
		status: "pending" | "resolved" | "rejected";
		value?: ILogMetadata;
		error?: Error;
	} = { status: "pending" };
	promise.then(
		(value) => Object.assign(outcome, { status: "resolved", value }),
		(error: Error) => Object.assign(outcome, { status: "rejected", error }),
	);
	return outcome;
}

async function start(appId: string, eventId: string) {
	let run!: ReturnType<typeof track>;
	await act(async () => {
		run = track(
			service.executeEvent(
				appId,
				eventId,
				{ id: "node" },
				false,
				undefined,
				undefined,
				false,
				LOAD,
			),
		);
	});
	await settle();
	return run;
}

async function settle() {
	await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
}

async function answer(action: () => unknown) {
	await act(async () => {
		await action();
	});
	await settle();
}

function dispatchedEvents() {
	return executeEventRemote.mock.calls.map(([, eventId]) => eventId);
}

beforeEach(async () => {
	for (const [key, value] of Object.entries(globals))
		Object.defineProperty(globalThis, key, {
			configurable: true,
			writable: true,
			value,
		});
	browser.localStorage.clear();
	prerunByEvent.clear();
	stored.clear();
	saveGate = undefined;
	runtimePrompt = undefined;
	wasmPrompt = undefined;
	executeEventRemote.mockClear();
	saveValues.mockClear();
	const container = browser.document.createElement("div");
	browser.document.body.append(container);
	root = createRoot(container as unknown as HTMLElement);
	await act(async () =>
		root.render(
			createElement(
				RuntimeVariablesProvider,
				{ value: store },
				createElement(ExecutionServiceProvider, null, createElement(Probe)),
			),
		),
	);
	mounted = true;
});

afterEach(async () => {
	if (mounted) await act(async () => root.unmount());
	mounted = false;
	await settle();
});

afterAll(async () => {
	for (const [path, actual] of actualModules) mock.module(path, () => actual);
	await browser.happyDOM.close();
	for (const [key, descriptor] of originals) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

test("the provider module exports only its component, so Fast Refresh can hot-swap it", () => {
	expect(Object.keys(providerModule)).toEqual(["ExecutionServiceProvider"]);
});

test("two runs that both need input get one prompt each, in order, and both settle", async () => {
	needsInput("first", "first-input");
	needsInput("second", "second-input");
	const first = await start("app-a", "first");
	const second = await start("app-b", "second");

	expect(runtimePrompt?.open).toBe(true);
	expect(runtimePrompt?.variables.map((v) => v.id)).toEqual(["first-input"]);

	await answer(() =>
		runtimePrompt?.onSave([{ variableId: "first-input", value: [49] }]),
	);
	expect(first.status).toBe("resolved");
	expect(second.status).toBe("pending");
	expect(runtimePrompt?.open).toBe(true);
	expect(runtimePrompt?.variables.map((v) => v.id)).toEqual(["second-input"]);

	await answer(() =>
		runtimePrompt?.onSave([{ variableId: "second-input", value: [50] }]),
	);
	expect(second.status).toBe("resolved");
	expect(runtimePrompt?.open).toBe(false);
	expect(dispatchedEvents()).toEqual(["first", "second"]);
	expect(executeEventRemote.mock.calls[1]?.[2].runtime_variables).toEqual({
		"second-input": expect.objectContaining({ default_value: [50] }),
	});
});

test("cancelling rejects only the run on screen; the next run's prompt follows", async () => {
	needsInput("first", "first-input");
	needsInput("second", "second-input");
	const first = await start("app-a", "first");
	const second = await start("app-b", "second");

	await answer(() => runtimePrompt?.onCancel());
	expect(first.status).toBe("rejected");
	expect(first.error?.message).toBe(CANCELLED);
	expect(second.status).toBe("pending");
	expect(runtimePrompt?.variables.map((v) => v.id)).toEqual(["second-input"]);

	await answer(() =>
		runtimePrompt?.onSave([{ variableId: "second-input", value: [50] }]),
	);
	expect(second.status).toBe("resolved");
	expect(dispatchedEvents()).toEqual(["second"]);
});

test("dismissing the dialog cancels the run instead of leaving it waiting forever", async () => {
	needsInput("first", "first-input");
	const first = await start("app-a", "first");

	await answer(() => runtimePrompt?.onOpenChange(false));
	expect(first.status).toBe("rejected");
	expect(first.error?.message).toBe(CANCELLED);
	expect(runtimePrompt?.open).toBe(false);
});

test("a queued run whose input the earlier prompt saved runs without asking again", async () => {
	needsInput("first", "shared-input");
	needsInput("second", "shared-input");
	const first = await start("app-a", "first");
	const second = await start("app-a", "second");

	await answer(() =>
		runtimePrompt?.onSave([{ variableId: "shared-input", value: [49] }]),
	);
	expect(first.status).toBe("resolved");
	expect(second.status).toBe("resolved");
	expect(runtimePrompt?.open).toBe(false);
	expect(executeEventRemote.mock.calls[1]?.[2].runtime_variables).toEqual({
		"shared-input": expect.objectContaining({ default_value: [49] }),
	});
});

test("cancelling also declines runs already waiting with the same question; a later run asks again", async () => {
	needsInput("tick", "shared-input");
	const first = await start("app-a", "tick");
	const queued = await start("app-a", "tick");

	await answer(() => runtimePrompt?.onCancel());
	expect(first.error?.message).toBe(CANCELLED);
	expect(queued.error?.message).toBe(CANCELLED);
	expect(runtimePrompt?.open).toBe(false);

	const later = await start("app-a", "tick");
	expect(runtimePrompt?.open).toBe(true);
	await answer(() =>
		runtimePrompt?.onSave([{ variableId: "shared-input", value: [49] }]),
	);
	expect(later.status).toBe("resolved");
	expect(dispatchedEvents()).toEqual(["tick"]);
});

test("cancelling WASM consent also declines the same run already waiting; a later run asks again", async () => {
	needsWasmConsent("tick", "pkg-a");
	const first = await start("app-a", "tick");
	const queued = await start("app-a", "tick");

	await answer(() => wasmPrompt?.onCancel());
	expect(first.status).toBe("resolved");
	expect(queued.status).toBe("resolved");
	expect(wasmPrompt?.open).toBe(false);

	const later = await start("app-a", "tick");
	expect(wasmPrompt?.open).toBe(true);
	await answer(() => wasmPrompt?.onConfirm("none"));
	expect(later.status).toBe("resolved");
	expect(dispatchedEvents()).toEqual(["tick"]);
});

test("cancel and dismiss during a save are ignored; the save settles the run once", async () => {
	needsInput("first", "first-input");
	let release!: () => void;
	saveGate = new Promise((resolve) => {
		release = resolve;
	});
	const first = await start("app-a", "first");

	let saving: Promise<void> | undefined;
	await act(async () => {
		saving = runtimePrompt?.onSave([
			{ variableId: "first-input", value: [49] },
		]);
	});
	await answer(() => runtimePrompt?.onCancel());
	await answer(() => runtimePrompt?.onOpenChange(false));
	expect(first.status).toBe("pending");
	expect(runtimePrompt?.open).toBe(true);

	await answer(async () => {
		release();
		await saving;
	});
	expect(first.status).toBe("resolved");
	expect(dispatchedEvents()).toEqual(["first"]);
	expect(runtimePrompt?.open).toBe(false);
});

test("two runs that both need WASM consent are asked in order; cancel answers only the first", async () => {
	needsWasmConsent("first", "pkg-a");
	needsWasmConsent("second", "pkg-b");
	const first = await start("app-a", "first");
	const second = await start("app-a", "second");

	expect(wasmPrompt?.open).toBe(true);
	expect(wasmPrompt?.packageIds).toEqual(["pkg-a"]);

	await answer(() => wasmPrompt?.onCancel());
	expect(first.status).toBe("resolved");
	expect(first.value).toBeUndefined();
	expect(second.status).toBe("pending");
	expect(wasmPrompt?.packageIds).toEqual(["pkg-b"]);

	await answer(() => wasmPrompt?.onConfirm("none"));
	expect(second.status).toBe("resolved");
	expect(wasmPrompt?.open).toBe(false);
	expect(dispatchedEvents()).toEqual(["second"]);
});

test("trusting a package on the first prompt lets a queued run with that package through", async () => {
	needsWasmConsent("first", "pkg-a");
	needsWasmConsent("second", "pkg-a");
	const first = await start("app-a", "first");
	const second = await start("app-a", "second");

	await answer(() => wasmPrompt?.onConfirm("package"));
	expect(first.status).toBe("resolved");
	expect(second.status).toBe("resolved");
	expect(wasmPrompt?.open).toBe(false);
	expect(dispatchedEvents()).toEqual(["first", "second"]);
});

test("unmounting the provider settles the prompt on screen and every queued one", async () => {
	needsInput("first", "first-input");
	needsInput("second", "second-input");
	needsWasmConsent("third", "pkg-a");
	needsWasmConsent("fourth", "pkg-b");
	const runs = [
		await start("app-a", "first"),
		await start("app-b", "second"),
		await start("app-a", "third"),
		await start("app-a", "fourth"),
	];

	await act(async () => root.unmount());
	mounted = false;
	await settle();

	const [first, second, third, fourth] = runs;
	expect(first.status).toBe("rejected");
	expect(first.error?.message).toBe(UNMOUNTED);
	expect(second.status).toBe("rejected");
	expect(second.error?.message).toBe(UNMOUNTED);
	expect(third.status).toBe("resolved");
	expect(fourth.status).toBe("resolved");
	expect(dispatchedEvents()).toEqual([]);
});
