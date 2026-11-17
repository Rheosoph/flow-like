import { describe, expect, test } from "bun:test";
import type { IBit } from "../schema";
import {
	bitModelName,
	describeModel,
	isBitIdLike,
	matchModelBit,
	modelLabel,
	providerDisplayName,
} from "./model-display-name";

function bit(partial: Record<string, unknown>): IBit {
	return { id: "bit", hub: "hub", meta: {}, ...partial } as unknown as IBit;
}

describe("isBitIdLike", () => {
	test("matches the id formats bits are created with", () => {
		for (const id of [
			"tz4a98xxat96iws9zmbrgj3a",
			"9f8b7c6d5e4f3a2b1c0d9e8f",
			"3f2a1b4c-5d6e-4f70-8a9b-0c1d2e3f4a5b",
			"3f2a1b4c5d6e4f708a9b0c1d2e3f4a5b",
		]) {
			expect(isBitIdLike(id)).toBe(true);
		}
	});

	test("does not match model names", () => {
		for (const name of [
			"gpt-4o-mini",
			"claude-opus-5",
			"openai/gpt-4o",
			"Qwen3-8B",
			"mlx-community/Qwen3-8B-4bit",
			"unknown",
		]) {
			expect(isBitIdLike(name)).toBe(false);
		}
	});
});

describe("bitModelName", () => {
	test("prefers the english catalog name", () => {
		const name = bitModelName(
			bit({
				meta: { en: { name: "GPT-5.6 Terra" }, de: { name: "Falsch" } },
				model_slug: "gpt-5-6-terra",
			}),
		);
		expect(name).toBe("GPT-5.6 Terra");
	});

	test("falls back to any locale, then evaluation, slug and provider model id", () => {
		expect(bitModelName(bit({ meta: { de: { name: "Nur DE" } } }))).toBe(
			"Nur DE",
		);
		expect(bitModelName(bit({ model_evaluation: { name: "Sonnet 5" } }))).toBe(
			"Sonnet 5",
		);
		expect(bitModelName(bit({ model_slug: "sonnet-5" }))).toBe("sonnet-5");
		expect(
			bitModelName(
				bit({ parameters: { provider: { model_id: "mlx-community/X" } } }),
			),
		).toBe("mlx-community/X");
	});

	test("ignores metadata that is itself an id", () => {
		expect(
			bitModelName(bit({ meta: { en: { name: "tz4a98xxat96iws9zmbrgj3a" } } })),
		).toBeUndefined();
	});

	test("returns undefined for a bit without any name", () => {
		expect(bitModelName(bit({}))).toBeUndefined();
		expect(bitModelName(undefined)).toBeUndefined();
	});
});

describe("modelLabel", () => {
	const names = new Map([["tz4a98xxat96iws9zmbrgj3a", "GPT-5.6 Terra"]]);

	test("renders the resolved bit name", () => {
		expect(modelLabel("tz4a98xxat96iws9zmbrgj3a", names)).toEqual({
			label: "GPT-5.6 Terra",
			resolved: true,
			opaque: false,
		});
	});

	test("shortens ids that could not be resolved", () => {
		expect(modelLabel("9f8b7c6d5e4f3a2b1c0d9e8f", names)).toEqual({
			label: "9f8b7c6d…",
			resolved: false,
			opaque: true,
		});
	});

	test("strips the provider prefix from plain model names", () => {
		expect(modelLabel("openai/gpt-4o").label).toBe("gpt-4o");
		expect(modelLabel("gpt-4o").label).toBe("gpt-4o");
	});

	test("labels missing models", () => {
		expect(modelLabel("").label).toBe("Unknown Model");
		expect(modelLabel(undefined).label).toBe("Unknown Model");
		expect(modelLabel("unknown").label).toBe("Unknown Model");
	});
});

function modelBit(id: string, name: string, provider: string, modelId: string) {
	return bit({
		id,
		meta: { en: { name } },
		parameters: { provider: { provider_name: provider, model_id: modelId } },
	});
}

describe("providerDisplayName", () => {
	test("keeps provider spellings and title-cases unknown keys", () => {
		expect(providerDisplayName("openai")).toBe("OpenAI");
		expect(providerDisplayName("LOCAL")).toBe("On-device");
		expect(providerDisplayName("fireworks")).toBe("Fireworks");
	});
});

describe("describeModel", () => {
	test("splits provider prefix and pinned snapshot off an unmatched model", () => {
		expect(describeModel("anthropic/claude-sonnet-4-5-20250929")).toMatchObject(
			{
				name: "claude-sonnet-4-5",
				provider: "Anthropic",
				snapshot: "2025-09-29",
				known: false,
				opaque: false,
			},
		);
		expect(describeModel("gpt-4o-2024-08-06")).toMatchObject({
			name: "gpt-4o",
			provider: undefined,
			snapshot: "2024-08-06",
		});
		expect(describeModel("claude-3-5-sonnet@20240620").snapshot).toBe(
			"2024-06-20",
		);
	});

	test("leaves names without a valid date alone", () => {
		expect(describeModel("qwen3-8b")).toMatchObject({
			name: "qwen3-8b",
			snapshot: undefined,
		});
		expect(describeModel("model-2024-13-40")).toMatchObject({
			name: "model-2024-13-40",
			snapshot: undefined,
		});
		expect(describeModel("mlx-community/Qwen3-8B-4bit")).toMatchObject({
			name: "Qwen3-8B-4bit",
			provider: undefined,
		});
	});

	test("uses the matched bit's catalog name, provider and location", () => {
		const local = modelBit(
			"tz4a98xxat96iws9zmbrgj3a",
			"Ling 3.0 Tiny",
			"Local",
			"",
		);
		expect(describeModel("tz4a98xxat96iws9zmbrgj3a", local)).toMatchObject({
			name: "Ling 3.0 Tiny",
			provider: "On-device",
			onDevice: true,
			known: true,
		});
		const hosted = modelBit(
			"b1",
			"Claude Sonnet 4.5",
			"anthropic",
			"claude-sonnet-4-5",
		);
		expect(describeModel("claude-sonnet-4-5-20250929", hosted)).toMatchObject({
			name: "Claude Sonnet 4.5",
			provider: "Anthropic",
			snapshot: "2025-09-29",
			onDevice: false,
		});
	});

	test("marks unresolved ids and missing models", () => {
		expect(describeModel("9f8b7c6d5e4f3a2b1c0d9e8f")).toMatchObject({
			name: "9f8b7c6d…",
			opaque: true,
		});
		expect(describeModel("unknown").missing).toBe(true);
		expect(describeModel(undefined).missing).toBe(true);
	});
});

describe("matchModelBit", () => {
	const sonnetDirect = modelBit(
		"b1",
		"Claude Sonnet 4.5",
		"anthropic",
		"claude-sonnet-4-5",
	);
	const sonnetRouter = modelBit(
		"b2",
		"Claude Sonnet 4.5",
		"openrouter",
		"anthropic/claude-sonnet-4-5",
	);
	const gpt = modelBit("b3", "GPT-4o", "openai", "gpt-4o");

	test("matches by bit id first", () => {
		expect(matchModelBit("b3", [sonnetDirect, gpt])).toBe(gpt);
	});

	test("matches the provider model id across snapshot dates and prefixes", () => {
		expect(matchModelBit("gpt-4o-2024-08-06", [sonnetDirect, gpt])).toBe(gpt);
		expect(matchModelBit("openai/gpt-4o", [sonnetDirect, gpt])).toBe(gpt);
	});

	test("prefers the exact model id when several bits serve the same model", () => {
		expect(
			matchModelBit("anthropic/claude-sonnet-4-5", [
				sonnetDirect,
				sonnetRouter,
			]),
		).toBe(sonnetRouter);
		expect(
			matchModelBit("anthropic/claude-sonnet-4-5-20250929", [
				sonnetRouter,
				sonnetDirect,
			]),
		).toBe(sonnetDirect);
	});

	test("returns nothing for unknown models", () => {
		expect(matchModelBit("mistral-large", [sonnetDirect, gpt])).toBeUndefined();
		expect(matchModelBit("", [gpt])).toBeUndefined();
	});
});
