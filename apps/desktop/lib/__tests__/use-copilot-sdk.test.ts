// @vitest-environment happy-dom
import type { AgentBackendProvider } from "@flow-like/flow-like-ui/components/flowpilot/types";
import { copilotBackendConnectionCoordinator } from "@flow-like/flow-like-ui/hooks/copilot-backend-coordinator";
import { useCopilotSDK } from "@flow-like/flow-like-ui/hooks/use-copilot-sdk";
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

const native = { invoke: vi.fn() };

type SDK = ReturnType<typeof useCopilotSDK>;
const backends: AgentBackendProvider[] = [
	"codex",
	"github-copilot",
	"claude-code",
];
const running = new Set<AgentBackendProvider>();
const renders: { backend: AgentBackendProvider; isRunning: boolean }[] = [];
let root: Root;
let sdk: SDK;

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((done, fail) => {
		resolve = done;
		reject = fail;
	});
	return { promise, resolve, reject };
}

function Harness({ backend }: { backend: AgentBackendProvider }) {
	sdk = useCopilotSDK(backend);
	renders.push({ backend, isRunning: sdk.isRunning });
	return null;
}

async function render(backend: AgentBackendProvider) {
	await act(async () => root.render(createElement(Harness, { backend })));
	await act(async () => vi.dynamicImportSettled());
}

function defaultInvoke(
	command: string,
	{ backend }: { backend: AgentBackendProvider },
) {
	switch (command) {
		case "flowpilot_agent_backend_is_running":
			return running.has(backend);
		case "flowpilot_agent_backend_start":
			running.add(backend);
			return;
		case "flowpilot_agent_backend_stop":
			running.delete(backend);
			return;
		case "flowpilot_agent_backend_list_models":
			if (backend === "github-copilot" && !running.has(backend)) {
				throw new Error("Copilot client not started");
			}
			return [{ id: `${backend}-model`, name: `${backend} model` }];
		case "flowpilot_agent_backend_get_auth_status":
			if (!running.has(backend)) throw new Error("Copilot client not started");
			return { authenticated: true, login: `${backend}-account` };
		default:
			throw new Error(`Unexpected command: ${command}`);
	}
}

beforeEach(async () => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	Object.assign(window, { __TAURI_INTERNALS__: { invoke: native.invoke } });
	for (const backend of backends) {
		await copilotBackendConnectionCoordinator.stop(
			backend,
			async () => undefined,
		);
	}
	running.clear();
	renders.length = 0;
	native.invoke.mockReset();
	native.invoke.mockImplementation(async (command, args) =>
		defaultInvoke(command, args),
	);
	root = createRoot(document.createElement("div"));
});

afterEach(async () => {
	await act(async () => root.unmount());
});

test("switching running Codex to stopped GitHub Copilot never probes its models or auth", async () => {
	running.add("codex");
	copilotBackendConnectionCoordinator.reconcile("codex", true);
	await render("codex");
	expect(sdk.isRunning).toBe(true);
	native.invoke.mockClear();

	await render("github-copilot");
	await act(async () => {
		await sdk.refreshModels();
		await sdk.refreshAuthStatus();
	});

	expect(
		native.invoke.mock.calls.map(([command, args]) => [command, args]),
	).toEqual([
		["flowpilot_agent_backend_is_running", { backend: "github-copilot" }],
	]);
	expect(
		renders
			.filter((entry) => entry.backend === "github-copilot")
			.every((entry) => !entry.isRunning),
	).toBe(true);
	expect(sdk).toMatchObject({
		isRunning: false,
		models: [],
		authStatus: null,
		error: null,
		hasLoadedModelCatalog: false,
	});
});

test("late models, auth and running status from the previous provider are ignored", async () => {
	const models = deferred<unknown[]>();
	const auth = deferred<{ authenticated: boolean; login: string }>();
	const status = deferred<boolean>();
	running.add("codex");
	copilotBackendConnectionCoordinator.reconcile("codex", true);
	native.invoke.mockImplementation(async (command, args) => {
		if (args.backend === "codex") {
			if (command === "flowpilot_agent_backend_list_models")
				return models.promise;
			if (command === "flowpilot_agent_backend_get_auth_status")
				return auth.promise;
			if (command === "flowpilot_agent_backend_is_running")
				return status.promise;
		}
		return defaultInvoke(command, args);
	});
	await render("codex");
	await render("claude-code");

	await act(async () => {
		models.resolve([{ id: "old-codex-model", name: "Old model" }]);
		auth.resolve({ authenticated: true, login: "old-codex-account" });
		status.resolve(false);
	});

	expect(sdk.models).toEqual([
		{ id: "claude-code-model", name: "claude-code model" },
	]);
	expect(sdk.authStatus).toBeNull();
	expect(sdk.error).toBeNull();
	expect(sdk.hasLoadedModelCatalog).toBe(true);
	expect(copilotBackendConnectionCoordinator.snapshot("codex").isRunning).toBe(
		true,
	);
});

test("late discovery failures cannot replace the selected provider state", async () => {
	const models = deferred<unknown[]>();
	const auth = deferred<unknown>();
	running.add("codex");
	copilotBackendConnectionCoordinator.reconcile("codex", true);
	native.invoke.mockImplementation(async (command, args) => {
		if (args.backend === "codex") {
			if (command === "flowpilot_agent_backend_list_models")
				return models.promise;
			if (command === "flowpilot_agent_backend_get_auth_status")
				return auth.promise;
		}
		return defaultInvoke(command, args);
	});
	await render("codex");
	await render("claude-code");
	await act(async () => {
		models.reject(new Error("Old Codex model failure"));
		auth.reject(new Error("Old Codex auth failure"));
	});
	expect(sdk.error).toBeNull();
	expect(sdk.models[0]?.id).toBe("claude-code-model");
	expect(sdk.hasLoadedModelCatalog).toBe(true);
});

test("returning to a provider does not revive requests from its earlier selection", async () => {
	const oldModels = deferred<unknown[]>();
	const oldAuth = deferred<unknown>();
	let firstSelection = true;
	running.add("codex");
	copilotBackendConnectionCoordinator.reconcile("codex", true);
	native.invoke.mockImplementation(async (command, args) => {
		if (args.backend === "codex" && firstSelection) {
			if (command === "flowpilot_agent_backend_list_models")
				return oldModels.promise;
			if (command === "flowpilot_agent_backend_get_auth_status")
				return oldAuth.promise;
		}
		return defaultInvoke(command, args);
	});
	await render("codex");
	await render("github-copilot");
	firstSelection = false;
	await render("codex");
	await act(async () => {
		oldModels.resolve([{ id: "obsolete", name: "Obsolete" }]);
		oldAuth.reject(new Error("Obsolete auth failure"));
	});
	expect(sdk.models[0]?.id).toBe("codex-model");
	expect(sdk.authStatus?.login).toBe("codex-account");
	expect(sdk.error).toBeNull();
});

test("stopping GitHub Copilot invalidates pending discovery and prevents new probes", async () => {
	const models = deferred<unknown[]>();
	const auth = deferred<unknown>();
	const stopped = deferred<void>();
	running.add("github-copilot");
	copilotBackendConnectionCoordinator.reconcile("github-copilot", true);
	native.invoke.mockImplementation(async (command, args) => {
		if (command === "flowpilot_agent_backend_list_models")
			return models.promise;
		if (command === "flowpilot_agent_backend_get_auth_status")
			return auth.promise;
		if (command === "flowpilot_agent_backend_stop") await stopped.promise;
		return defaultInvoke(command, args);
	});
	await render("github-copilot");
	let stopping!: Promise<void>;
	await act(async () => {
		stopping = sdk.stop();
	});
	await act(async () => vi.dynamicImportSettled());
	native.invoke.mockClear();
	await act(async () => {
		await sdk.refreshModels();
		await sdk.refreshAuthStatus();
	});
	expect(native.invoke).not.toHaveBeenCalled();
	await act(async () => {
		stopped.resolve();
		await stopping;
		models.resolve([{ id: "obsolete", name: "Obsolete" }]);
		auth.reject(new Error("Obsolete auth failure"));
	});
	expect(sdk).toMatchObject({
		isRunning: false,
		models: [],
		authStatus: null,
		hasLoadedModelCatalog: false,
		error: null,
	});
});

test("successful GitHub startup loads models and auth after the native client is ready", async () => {
	const startup = deferred<void>();
	native.invoke.mockImplementation(async (command, args) => {
		if (command === "flowpilot_agent_backend_start") {
			await startup.promise;
			running.add(args.backend);
			return;
		}
		return defaultInvoke(command, args);
	});
	await render("github-copilot");
	let started!: Promise<void>;
	await act(async () => {
		started = sdk.start();
	});
	await act(async () => vi.dynamicImportSettled());
	expect(sdk.isConnecting).toBe(true);
	expect(native.invoke.mock.calls.map(([command]) => command)).toEqual([
		"flowpilot_agent_backend_is_running",
		"flowpilot_agent_backend_start",
	]);

	await act(async () => {
		startup.resolve();
		await started;
	});
	await act(async () => vi.dynamicImportSettled());
	expect(sdk).toMatchObject({
		isRunning: true,
		isConnecting: false,
		models: [{ id: "github-copilot-model", name: "github-copilot model" }],
		authStatus: { authenticated: true, login: "github-copilot-account" },
		hasLoadedModelCatalog: true,
		error: null,
	});
});

test("a delayed stopped probe cannot overwrite a completed startup", async () => {
	const status = deferred<boolean>();
	native.invoke.mockImplementation(async (command, args) => {
		if (command === "flowpilot_agent_backend_is_running") return status.promise;
		return defaultInvoke(command, args);
	});
	await render("github-copilot");
	await act(async () => sdk.start());
	await act(async () => vi.dynamicImportSettled());
	await act(async () => status.resolve(false));
	expect(sdk.isRunning).toBe(true);
	expect(sdk.hasLoadedModelCatalog).toBe(true);
	expect(sdk.authStatus?.authenticated).toBe(true);
});
