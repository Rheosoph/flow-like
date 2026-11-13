import { describe, expect, test } from "bun:test";
import type { GlobalChatAgentSelection } from "../../state/global-chat/global-chat-store";
import {
	assistantSelection,
	resolveModelSelection,
	selectedModelLabel,
	withAssistantModel,
} from "./model-selection";

const fallbackModels = [{ id: "default" }];
const liveModels = [{ id: "gpt-5.6-terra" }, { id: "gpt-5.6-sol" }];

describe("resolveModelSelection", () => {
	test("does not let a fallback catalog overwrite a live selection", () => {
		expect(
			resolveModelSelection({
				models: fallbackModels,
				selectedModelId: "gpt-5.6-terra",
				rememberedModelId: "gpt-5.6-terra",
				canReplaceInvalidSelection: false,
			}),
		).toBeNull();
	});

	test("lets a fallback catalog initialize an empty selection", () => {
		expect(
			resolveModelSelection({
				models: fallbackModels,
				selectedModelId: "",
				rememberedModelId: null,
				canReplaceInvalidSelection: false,
			}),
		).toBe("default");
	});

	test("restores a remembered model when the live catalog arrives", () => {
		expect(
			resolveModelSelection({
				models: liveModels,
				selectedModelId: "default",
				rememberedModelId: "gpt-5.6-sol",
				canReplaceInvalidSelection: true,
			}),
		).toBe("gpt-5.6-sol");
	});

	test("repairs an invalid selection from an authoritative catalog", () => {
		expect(
			resolveModelSelection({
				models: liveModels,
				selectedModelId: "removed-model",
				rememberedModelId: null,
				canReplaceInvalidSelection: true,
			}),
		).toBe("gpt-5.6-terra");
	});
});

describe("assistantSelection", () => {
	const picker = {
		provider: "codex",
		selectedModelId: "gpt-5.6-sol",
		reasoningEffort: "high",
	} as const;

	test("runs assistants on the profile model the bot turn names", () => {
		expect(assistantSelection(picker, "bit-haiku")).toEqual({
			provider: "bits",
			selectedModelId: "bit-haiku",
			reasoningEffort: "high",
		});
	});

	test("keeps the run's own selection outside a bot turn", () => {
		expect(assistantSelection(picker)).toBe(picker);
		expect(assistantSelection(picker, "")).toBe(picker);
	});
});

describe("selectedModelLabel", () => {
	const models = [
		{ id: "bit-sonnet", name: "Claude Sonnet" },
		{ id: "gpt-5.6-sol", name: "" },
	];

	test("is the selected model's name", () => {
		expect(selectedModelLabel(models, "bit-sonnet")).toBe("Claude Sonnet");
	});

	test("falls back to the id of a model without a name", () => {
		expect(selectedModelLabel(models, "gpt-5.6-sol")).toBe("gpt-5.6-sol");
	});

	test("is null with nothing selected or a selection the catalog doesn't hold yet", () => {
		expect(selectedModelLabel(models, "")).toBeNull();
		expect(selectedModelLabel(models, "bit-retired")).toBeNull();
		expect(selectedModelLabel([], "bit-sonnet")).toBeNull();
	});
});

describe("withAssistantModel", () => {
	const bits: GlobalChatAgentSelection = {
		provider: "bits",
		selectedModelId: "bit-1",
		reasoningEffort: "",
	};

	test("an error result names the model the assistant ran on", () => {
		expect(
			withAssistantModel({ status: "error", message: "boom" }, bits),
		).toEqual({ status: "error", message: "boom", model_id: "bit-1" });
		expect(
			withAssistantModel(
				{ status: "error" },
				{ ...bits, provider: "codex", selectedModelId: "gpt" },
			),
		).toEqual({ status: "error", model_id: "codex:gpt" });
	});

	test("successful and foreign results stay unchanged", () => {
		const ok = { status: "ok" };
		expect(withAssistantModel(ok, bits)).toBe(ok);
		expect(withAssistantModel(undefined, bits)).toBeUndefined();
		const named = { status: "error", model_id: "kept" };
		expect(withAssistantModel(named, bits)).toBe(named);
	});
});
