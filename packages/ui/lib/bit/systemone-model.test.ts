import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema/bit/bit";
import {
	SYSTEMONE_HOSTED_PROVIDERS,
	createSystemOneParameters,
	updateSystemOneBit,
	validateSystemOneBit,
	validateSystemOneParameters,
} from "./systemone-model";

const fixture = (): IBit =>
	({
		type: IBitTypes.SystemOne,
		download_link: "https://example.test/model.gguf",
		file_name: "model.gguf",
		size: 120,
		dependencies: ["hub:old"],
		parameters: createSystemOneParameters(),
	}) as IBit;

describe("SystemOne authoring", () => {
	test("accepts the native provider contract and rejects chat API settings", () => {
		expect(validateSystemOneParameters(createSystemOneParameters())).toBeNull();
		for (const { value } of SYSTEMONE_HOSTED_PROVIDERS) {
			const parameters = {
				context_length: 512,
				provider: {
					provider_name: value,
					model_id: "decisions",
					params: { tier: "FREE" },
				},
			};
			expect(validateSystemOneParameters(parameters)).toBeNull();
			expect(
				validateSystemOneParameters({
					...parameters,
					provider: { ...parameters.provider, api_surface: "SystemOne" },
				}),
			).toBeNull();
			expect(
				validateSystemOneParameters({
					...parameters,
					provider: { ...parameters.provider, api_surface: "ChatCompletions" },
				}),
			).toContain("native API");
			expect(
				validateSystemOneParameters({
					...parameters,
					provider: { ...parameters.provider, params: { api_key: "secret" } },
				}),
			).toContain("server");
		}
		expect(
			validateSystemOneParameters({
				context_length: 512,
				provider: { provider_name: "hosted:openai", model_id: "chat" },
			}),
		).toContain("Chat providers");
	});
	test("keeps custom endpoints private and validates their URL", () => {
		const parameters = {
			context_length: 512,
			provider: {
				provider_name: "custom:systemone",
				model_id: "decisions",
				params: { endpoint: "https://models.example.test", api_key: "secret" },
			},
		};
		expect(validateSystemOneParameters(parameters, "custom")).toBeNull();
		expect(validateSystemOneParameters(parameters, "admin")).not.toBeNull();
		for (const endpoint of [
			"file:///tmp/secret",
			"https://key@example.test",
			"https://example.test?key=value",
			"https://example.test#route",
		]) {
			parameters.provider.params.endpoint = endpoint;
			expect(validateSystemOneParameters(parameters, "custom")).toContain(
				"HTTP or HTTPS",
			);
		}
	});
	test("checks context bounds and local GGUF artifacts before uploading", () => {
		for (const context_length of [0, -1, 1.5, 2 ** 32, "512"])
			expect(
				validateSystemOneParameters({
					...createSystemOneParameters(),
					context_length,
				}),
			).not.toBeNull();
		expect(validateSystemOneBit(fixture())).toBeNull();
		expect(
			validateSystemOneBit(fixture(), {
				...fixture(),
				file_name: "mmproj.gguf",
			}),
		).toBeNull();
		expect(
			validateSystemOneBit(fixture(), { ...fixture(), download_link: null }),
		).toContain("optional SystemOne image projector");
		expect(
			validateSystemOneBit({ ...fixture(), file_name: "model.safetensors" }),
		).toContain("GGUF");
	});
	test("switching to hosted removes file fields and stale dependencies", () => {
		const next = updateSystemOneBit(fixture(), {
			context_length: 512,
			provider: { provider_name: "hosted:openrouter", model_id: "decisions" },
		});
		expect(next.download_link).toBeNull();
		expect(next.file_name).toBeNull();
		expect(next.size).toBe(0);
		expect(next.dependencies).toEqual([]);
		expect(validateSystemOneBit(next)).toBeNull();
	});
});
