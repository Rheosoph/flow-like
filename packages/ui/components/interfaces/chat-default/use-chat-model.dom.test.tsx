// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { type IBit, IBitTypes } from "../../../lib/schema/bit/bit";
import { IEventExecutionMode } from "../../../lib/schema/flow/event";
import type { IEventPayloadChat } from "../../../lib/schema/flow/event-payload-chat";
import { useChatModel } from "./use-chat-model";

const state = vi.hoisted(() => ({
	bits: [] as IBit[],
	refs: [] as string[],
	read: vi.fn(),
	store: vi.fn(async (_appId: string, _model: string | undefined) => {}),
	capabilities: {
		canHostLlamaCPP: true,
		canHostMLX: true,
		canUseNativeAgentProviders: true,
		deviceModels: true,
	},
}));

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (_key: string, fallback: string) => fallback,
		i18n: { language: "en" },
	}),
}));
vi.mock("../../../state/backend-state", () => ({
	useBackend: () => ({
		userState: { getSettingsProfile: function getSettingsProfile() {} },
		bitState: { getProfileBits: function getProfileBits() {} },
		capabilities: () => state.capabilities,
	}),
}));
vi.mock("../../../hooks/use-invoke", () => ({
	useInvoke: (fn: { name: string }) => ({
		data:
			fn.name === "getSettingsProfile"
				? { hub_profile: { id: "profile", bits: state.refs } }
				: state.bits,
	}),
}));
vi.mock("./chat-model-selection", async (original) => ({
	...(await original<typeof import("./chat-model-selection")>()),
	readChatModel: (appId: string) => state.read(appId),
	storeChatModel: (appId: string, model: string | undefined) =>
		state.store(appId, model),
}));

let root: Root;
let container: HTMLDivElement;
let current: ReturnType<typeof useChatModel>;
let renderedSelections: (string | undefined)[];

function Probe({
	appId,
	config,
	executionMode,
}: {
	appId: string;
	config?: Partial<IEventPayloadChat>;
	executionMode?: IEventExecutionMode;
}) {
	current = useChatModel(appId, config, executionMode);
	renderedSelections.push(current.selectedModel);
	return null;
}

async function render(
	appId = "app-a",
	config?: Partial<IEventPayloadChat>,
	executionMode?: IEventExecutionMode,
) {
	await act(async () => {
		root.render(
			<Probe appId={appId} config={config} executionMode={executionMode} />,
		);
	});
}

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	state.read.mockReset().mockResolvedValue(undefined);
	state.store.mockClear();
	state.bits = [
		{ id: "text", type: IBitTypes.Llm },
		{ id: "vision", type: IBitTypes.Vlm },
		{
			id: "local",
			type: IBitTypes.Llm,
			parameters: { provider: { provider_name: "Local" } },
		},
	] as IBit[];
	state.refs = ["hub:text", "hub:vision", "hub:local"];
	renderedSelections = [];
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.unstubAllGlobals();
});

describe("app chat model selection", () => {
	test("is enabled by default, remembers selections and supports Automatic", async () => {
		await render();
		expect(current.picker?.value).toBe("");
		await act(async () => current.picker?.onChange("vision"));
		expect(current.selectedModel).toBe("vision");
		expect(state.store).toHaveBeenLastCalledWith("app-a", "vision");
		await act(async () => current.picker?.onChange(""));
		expect(current.selectedModel).toBeUndefined();
		expect(state.store).toHaveBeenLastCalledWith("app-a", undefined);
	});

	test("omits unavailable, non-vision and disabled selections without overwriting storage", async () => {
		state.read.mockResolvedValue("text");
		await render();
		expect(current.selectedModel).toBe("text");
		await render("app-a", { require_vlm: true });
		expect(current.selectedModel).toBeUndefined();
		expect(current.picker?.options.map((option) => option.id)).toEqual([
			"",
			"vision",
		]);
		await render("app-a", { allow_model_selection: false });
		expect(current.selectedModel).toBeUndefined();
		expect(current.picker).toBeUndefined();
		state.refs = ["hub:vision"];
		await render();
		expect(current.selectedModel).toBeUndefined();
		expect(state.store).not.toHaveBeenCalled();
	});

	test("excludes local models for an event pinned to Remote", async () => {
		state.read.mockResolvedValue("local");
		await render();
		expect(current.selectedModel).toBe("local");
		await render("app-a", {}, IEventExecutionMode.Remote);
		expect(current.selectedModel).toBeUndefined();
		expect(current.picker?.options.map((option) => option.id)).toEqual([
			"",
			"text",
			"vision",
		]);
	});

	test("never sends the previous app's choice while the next one restores", async () => {
		let restoreSecond: (value: string) => void = () => {};
		state.read.mockImplementation((appId) =>
			appId === "app-a"
				? Promise.resolve("text")
				: new Promise<string>((resolve) => {
						restoreSecond = resolve;
					}),
		);
		await render();
		expect(current.selectedModel).toBe("text");
		renderedSelections = [];
		await render("app-b");
		expect(renderedSelections).not.toContain("text");
		await act(async () => restoreSecond("vision"));
		expect(current.selectedModel).toBe("vision");
	});

	test("a late restore cannot overwrite a newer pick", async () => {
		let restoreFirst: (value: string) => void = () => {};
		state.read.mockImplementation((appId) =>
			appId === "app-a"
				? new Promise<string>((resolve) => {
						restoreFirst = resolve;
					})
				: Promise.resolve("vision"),
		);
		await render();
		await act(async () => current.picker?.onChange("vision"));
		await act(async () => restoreFirst("text"));
		expect(current.selectedModel).toBe("vision");
	});

	test("a late restore cannot overwrite a different app's selection", async () => {
		let restoreFirst: (value: string) => void = () => {};
		state.read.mockImplementation((appId) =>
			appId === "app-a"
				? new Promise<string>((resolve) => {
						restoreFirst = resolve;
					})
				: Promise.resolve("vision"),
		);
		await render();
		await render("app-b");
		await act(async () => restoreFirst("text"));
		expect(current.selectedModel).toBe("vision");
	});
});
