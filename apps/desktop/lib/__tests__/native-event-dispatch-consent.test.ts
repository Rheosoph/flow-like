import {
	type ExecutionServiceContextValue,
	ExecutionServiceProvider,
	useExecutionService,
} from "@flow-like/flow-like-ui/state/execution-service-context";
// @vitest-environment happy-dom
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
const mocks = vi.hoisted(() => ({
	backend: {
		eventState: {
			alwaysRemote: true,
			prerunEvent: vi.fn(),
			executeEvent: vi.fn(),
			executeEventRemote: vi.fn(),
		},
		boardState: {},
	},
	variables: { hasAllValues: vi.fn(), getValues: vi.fn(), saveValues: vi.fn() },
	prompt: undefined as
		| undefined
		| { open: boolean; onSave: (values: unknown[]) => Promise<void> },
	wasm: undefined as
		| undefined
		| { open: boolean; onConfirm: (remember: "none") => void },
}));
vi.mock("@flow-like/flow-like-ui/state/backend-state", () => ({
	useBackend: () => mocks.backend,
}));
vi.mock("@flow-like/flow-like-ui/state/runtime-variables-context", () => ({
	useRuntimeVariables: () => mocks.variables,
}));
vi.mock("@flow-like/flow-like-ui/state/backend-state/prerun-cache", () => ({
	prerunEventKey: () => "event",
	prerunBoardKey: () => "board",
	prerunSwr: (_key: string, fetch: () => Promise<unknown>) => fetch(),
}));
vi.mock(
	"@flow-like/flow-like-ui/components/flow/runtime-variables-prompt",
	() => ({
		RuntimeVariablesPrompt: (props: typeof mocks.prompt) => {
			mocks.prompt = props;
			return null;
		},
	}),
);
vi.mock(
	"@flow-like/flow-like-ui/components/flow/wasm-sandbox-warning-dialog",
	() => ({
		WasmSandboxWarningDialog: (props: typeof mocks.wasm) => {
			mocks.wasm = props;
			return null;
		},
	}),
);
vi.mock("sonner", () => ({ toast: { warning: vi.fn() } }));
let root: Root;
let service: ExecutionServiceContextValue;
let current: boolean;
const guard = () => {
	if (!current) throw new Error("Native scope changed or deadline expired");
};
const prerun = {
	board_id: "board",
	can_execute_locally: false,
	execution_mode: "Remote",
	event_execution_mode: "Remote",
	has_wasm_nodes: false,
	runtime_variables: [
		{
			id: "input",
			name: "Input",
			data_type: "String",
			value_type: "Normal",
			secret: false,
		},
	],
};
function start() {
	return service.executeEvent(
		"app",
		"event",
		{ id: "node" },
		false,
		undefined,
		undefined,
		false,
		undefined,
		guard,
	);
}
beforeEach(async () => {
	vi.clearAllMocks();
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	current = true;
	mocks.prompt = undefined;
	mocks.wasm = undefined;
	mocks.backend.eventState.prerunEvent.mockResolvedValue(prerun);
	mocks.variables.hasAllValues.mockResolvedValue(false);
	mocks.variables.getValues.mockResolvedValue(new Map());
	mocks.variables.saveValues.mockResolvedValue(undefined);
	root = createRoot(document.createElement("div"));
	function Probe() {
		service = useExecutionService();
		return null;
	}
	await act(async () =>
		root.render(
			createElement(ExecutionServiceProvider, null, createElement(Probe)),
		),
	);
});
afterEach(async () => {
	await act(async () => root.unmount());
});
test("a native request cannot save or run after identity changes while the runtime input prompt is open", async () => {
	let pending!: Promise<unknown>;
	await act(async () => {
		pending = start().catch((error: Error) => error);
	});
	expect(mocks.prompt?.open).toBe(true);
	current = false;
	await act(async () => {
		await mocks.prompt?.onSave([{ variableId: "input", value: [34, 120, 34] }]);
	});
	expect(await pending).toEqual(
		new Error("Native scope changed or deadline expired"),
	);
	expect(mocks.variables.saveValues).not.toHaveBeenCalled();
	expect(mocks.backend.eventState.executeEvent).not.toHaveBeenCalled();
	expect(mocks.backend.eventState.executeEventRemote).not.toHaveBeenCalled();
	expect(mocks.prompt?.open).toBe(false);
});
test("identity is checked again after saving the entered runtime values", async () => {
	const saved = Promise.withResolvers<void>();
	mocks.variables.saveValues.mockReturnValue(saved.promise);
	let pending!: Promise<unknown>;
	await act(async () => {
		pending = start().catch((error: Error) => error);
	});
	let saving: Promise<void> | undefined;
	await act(async () => {
		saving = mocks.prompt?.onSave([
			{ variableId: "input", value: [34, 120, 34] },
		]);
	});
	expect(mocks.variables.saveValues).toHaveBeenCalledTimes(1);
	current = false;
	await act(async () => {
		saved.resolve();
		await saving;
	});
	expect(await pending).toBeInstanceOf(Error);
	expect(mocks.backend.eventState.executeEventRemote).not.toHaveBeenCalled();
});
test("expiration during asynchronous prerun rejects before prompting or dispatching", async () => {
	const preparation = Promise.withResolvers<typeof prerun>();
	mocks.backend.eventState.prerunEvent.mockReturnValue(preparation.promise);
	let pending!: Promise<unknown>;
	await act(async () => {
		pending = start().catch((error: Error) => error);
	});
	current = false;
	await act(async () => {
		preparation.resolve(prerun);
	});
	expect(await pending).toBeInstanceOf(Error);
	expect(mocks.prompt?.open).toBe(false);
	expect(mocks.backend.eventState.executeEventRemote).not.toHaveBeenCalled();
});
test("confirming WASM consent after a native deadline cannot dispatch the Event", async () => {
	mocks.backend.eventState.prerunEvent.mockResolvedValue({
		...prerun,
		has_wasm_nodes: true,
		wasm_package_ids: ["untrusted"],
	});
	let pending!: Promise<unknown>;
	await act(async () => {
		pending = start().catch((error: Error) => error);
	});
	expect(mocks.wasm?.open).toBe(true);
	current = false;
	await act(async () => {
		mocks.wasm?.onConfirm("none");
	});
	expect(await pending).toBeInstanceOf(Error);
	expect(mocks.prompt?.open).toBe(false);
	expect(mocks.backend.eventState.executeEventRemote).not.toHaveBeenCalled();
});
test("valid input consent keeps the final identity guard on the selected remote transport", async () => {
	let pending!: Promise<unknown>;
	await act(async () => {
		pending = start();
	});
	await act(async () => {
		await mocks.prompt?.onSave([{ variableId: "input", value: [34, 120, 34] }]);
	});
	await pending;
	expect(mocks.backend.eventState.executeEventRemote).toHaveBeenCalledWith(
		"app",
		"event",
		expect.objectContaining({
			runtime_variables: {
				input: expect.objectContaining({ default_value: [34, 120, 34] }),
			},
		}),
		false,
		undefined,
		undefined,
		undefined,
		guard,
	);
});
