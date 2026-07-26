import { describe, expect, test } from "bun:test";
import { resolveModelSelection } from "./model-selection";

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
