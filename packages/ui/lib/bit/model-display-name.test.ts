import { describe, expect, test } from "bun:test";
import type { IBit } from "../schema";
import { bitModelName, isBitIdLike, modelLabel } from "./model-display-name";

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
