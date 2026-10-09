import { expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../../../../lib/schema/bit/bit";
import { modelAccess } from "./model-access";

const model = (type: IBitTypes, provider: unknown, remote?: unknown): IBit =>
	({ type, parameters: { provider, remote } }) as IBit;

test("local text embeddings only offer hosted fallback with a remote model identity", () => {
	const local = { provider_name: " LoCaL " };
	expect(modelAccess(model(IBitTypes.Embedding, local))).toBe("local");
	expect(modelAccess(model(IBitTypes.Embedding, local, {}))).toBe("local");
	expect(
		modelAccess(model(IBitTypes.Embedding, local, { model_id: "  " })),
	).toBe("local");
	expect(
		modelAccess(model(IBitTypes.Embedding, local, { model_id: "embed" })),
	).toBe("local_with_hosted_fallback");
	expect(
		modelAccess(
			model(IBitTypes.Embedding, { ...local, model_id: "embed" }, {}),
		),
	).toBe("local_with_hosted_fallback");
	// A provider model ID alone does not enable the proxy for a Local model.
	expect(
		modelAccess(model(IBitTypes.Embedding, { ...local, model_id: "embed" })),
	).toBe("local");
});

test("image embeddings and local completion models have no hosted fallback", () => {
	const remote = { model_id: "future-model" };
	expect(
		modelAccess(
			model(IBitTypes.ImageEmbedding, { provider_name: " local " }, remote),
		),
	).toBe("local");
	for (const type of [IBitTypes.Llm, IBitTypes.Vlm]) {
		for (const provider_name of ["local", " MLX "]) {
			expect(modelAccess(model(type, { provider_name }, remote))).toBe("local");
		}
	}
});

test("decision models distinguish local execution from hosted approval", () => {
	expect(
		modelAccess(model(IBitTypes.SystemOne, { provider_name: "Local" })),
	).toBe("local");
	expect(
		modelAccess(
			model(IBitTypes.SystemOne, { provider_name: "hosted:typesafe" }),
		),
	).toBe("hosted");
});

test("hosted aliases and embedding proxy configuration are recognized", () => {
	for (const provider_name of [
		"Hosted",
		" HOSTED:OpenAI ",
		"Premium",
		" internal ",
	]) {
		expect(modelAccess(model(IBitTypes.Llm, { provider_name }))).toBe("hosted");
		expect(
			modelAccess(
				model(IBitTypes.Embedding, { provider_name, model_id: "embed" }),
			),
		).toBe("hosted");
	}
	expect(
		modelAccess(
			model(
				IBitTypes.Embedding,
				{ provider_name: "OpenAI" },
				{ model_id: "embed" },
			),
		),
	).toBe("hosted");
});

test("unrecognized or malformed metadata does not claim on-device capability", () => {
	for (const provider of [
		undefined,
		null,
		[],
		{},
		{ provider_name: 7 },
		{ provider_name: "future-runtime" },
	]) {
		expect(modelAccess(model(IBitTypes.Llm, provider))).toBe("unknown");
	}
	expect(
		modelAccess(model(IBitTypes.Embedding, { provider_name: "Local" }, [])),
	).toBe("unknown");
	expect(modelAccess(model(IBitTypes.File, { provider_name: "Local" }))).toBe(
		"unknown",
	);
	expect(
		modelAccess(model(IBitTypes.Embedding, { provider_name: "Hosted" })),
	).toBe("unknown");
});
