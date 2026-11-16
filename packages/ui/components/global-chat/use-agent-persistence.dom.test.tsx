// @vitest-environment happy-dom
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

let root: Root;
let host: HTMLDivElement;

async function session() {
	const store = await import("../../state/global-chat/global-chat-store");
	const { useHydrateAgentSelection } = await import("./use-agent-persistence");
	function Hydrate() {
		useHydrateAgentSelection();
		return null;
	}
	return { ...store, Hydrate };
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.resetModules();
	localStorage.clear();
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	vi.restoreAllMocks();
	localStorage.clear();
});

test.each(["embed-a", ""])(
	"restores the explicit memory selection %j after a reload",
	async (modelId) => {
		const before = await session();
		before.useGlobalChatStore.getState().selectEmbeddingModel(modelId);
		expect(localStorage.getItem(before.AGENT_EMBEDDING_MODEL_KEY)).toBe(
			modelId,
		);

		vi.resetModules();
		const after = await session();
		// An explicit off selection must also replace a transient in-session value.
		after.useGlobalChatStore.getState().setEmbeddingModelId("temporary-model");
		const write = vi.spyOn(localStorage, "setItem");
		await act(async () => root.render(<after.Hydrate />));
		expect(after.useGlobalChatStore.getState().embeddingModelId).toBe(modelId);
		expect(write).not.toHaveBeenCalled();
	},
);

test("opening another chat surface keeps the current memory selection", async () => {
	const first = await session();
	localStorage.setItem(first.AGENT_EMBEDDING_MODEL_KEY, "embed-a");
	await act(async () => root.render(<first.Hydrate />));
	first.useGlobalChatStore.getState().setEmbeddingModelId("embed-b");
	await act(async () => root.render(null));
	await act(async () => root.render(<first.Hydrate />));
	expect(first.useGlobalChatStore.getState().embeddingModelId).toBe("embed-b");
});

test("restores a custom Claude Bit as a Profile selection after reload", async () => {
	const before = await session();
	before.useGlobalChatStore.getState().selectProvider("bits");
	before.useGlobalChatStore.getState().selectModel("custom-claude-code");
	vi.resetModules();
	const after = await session();
	after.useGlobalChatStore.getState().setProvider("claude-code");
	after.useGlobalChatStore.getState().setSelectedModelId("default");
	await act(async () => root.render(<after.Hydrate />));
	expect(after.useGlobalChatStore.getState()).toMatchObject({
		provider: "bits",
		selectedModelId: "custom-claude-code",
	});
	await act(async () => root.render(null));
	after.useGlobalChatStore.getState().selectModel("another-custom-bit");
	await act(async () => root.render(<after.Hydrate />));
	expect(after.useGlobalChatStore.getState().selectedModelId).toBe(
		"another-custom-bit",
	);
});

test.each(["current", "legacy"])(
	"a provider switch cannot hydrate the previous provider's Bit from %s preferences",
	async (preferences) => {
		const before = await session();
		before.useGlobalChatStore.getState().selectProvider("bits");
		before.useGlobalChatStore.getState().selectModel("custom-claude-code");
		if (preferences === "legacy") {
			localStorage.removeItem(before.AGENT_MODEL_PROVIDER_KEY);
		}
		before.useGlobalChatStore.getState().selectProvider("codex");
		vi.resetModules();
		const after = await session();
		await act(async () => root.render(<after.Hydrate />));
		expect(after.useGlobalChatStore.getState()).toMatchObject({
			provider: "codex",
			selectedModelId: "",
		});
	},
);

test("hydrates a legacy provider and model preference without a model-provider key", async () => {
	const current = await session();
	localStorage.setItem(current.AGENT_PROVIDER_KEY, "bits");
	localStorage.setItem(current.AGENT_MODEL_KEY, "legacy-custom-claude");
	await act(async () => root.render(<current.Hydrate />));
	expect(current.useGlobalChatStore.getState()).toMatchObject({
		provider: "bits",
		selectedModelId: "legacy-custom-claude",
	});
});

test("memory selection still works when browser storage is unavailable", async () => {
	const { useGlobalChatStore, Hydrate } = await session();
	vi.spyOn(localStorage, "setItem").mockImplementation(() => {
		throw new Error("Storage unavailable");
	});
	vi.spyOn(localStorage, "getItem").mockImplementation(() => {
		throw new Error("Storage unavailable");
	});
	useGlobalChatStore.getState().selectEmbeddingModel("embed-a");
	await act(async () => root.render(<Hydrate />));
	expect(useGlobalChatStore.getState().embeddingModelId).toBe("embed-a");
});
