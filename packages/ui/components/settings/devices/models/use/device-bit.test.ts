import { describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../../../lib/api-error";
import {
	isDeviceModelBit,
	isHostableLlmModel,
} from "../../../../../lib/bit/local-model-filter";
import { IBitTypes } from "../../../../../lib/schema/bit/bit";
import type { IBackendState } from "../../../../../state/backend-state";
import {
	DeviceBitError,
	defaultContext,
	deviceBitFailure,
	deviceBitIdFor,
	deviceModelBit,
	saveDeviceModelBit,
	savedDeviceBit,
	wholeNumber,
} from "./device-bit";

const DEVICE = "5b794764-6afc-4ac9-89c2-c6d9eb91fd42";
const NOW = Date.UTC(2026, 9, 5, 12);

const qwen = { id: "qwen3-8b-q4", kind: "chat" as const };

function bitFor(
	model: { id: string; kind: "chat" | "vision" | "embedding" },
	extra: { vectorLength?: number } = {},
) {
	return deviceModelBit({
		id: `bit-${model.id}`,
		deviceId: DEVICE,
		model,
		name: "Qwen3 8B on gpu-box",
		description: "Chat model hosted on gpu-box.",
		contextLength: 16_384,
		now: NOW,
		...extra,
	});
}

describe("device model Bit", () => {
	test("a chat model becomes an LLM Bit of the device provider, with no link, file or secret", () => {
		const bit = bitFor(qwen);
		expect(bit.type).toBe(IBitTypes.Llm);
		expect(bit.parameters).toEqual({
			context_length: 16_384,
			provider: {
				provider_name: "device",
				model_id: "qwen3-8b-q4",
				version: null,
				params: {
					device_id: DEVICE,
					model: "qwen3-8b-q4",
					kind: "chat",
					api_surface: "chat_completions",
				},
			},
			model_classification: expect.objectContaining({
				reasoning: 0.5,
				cost: 0.5,
			}),
		});
		expect(Object.values(bit.parameters.model_classification)).not.toContain(0);
		expect(bit).toMatchObject({
			download_link: null,
			file_name: null,
			size: null,
			dependencies: [],
			hub: "",
			created: "2026-10-05T12:00:00.000Z",
		});
		expect(bit.meta.en.name).toBe("Qwen3 8B on gpu-box");
		expect(bit.meta.en.created_at.secs_since_epoch).toBe(NOW / 1000);
		expect(JSON.stringify(bit)).not.toMatch(/api_key|token|secret/i);
		expect(isDeviceModelBit(bit)).toBe(true);
	});

	test("vision and embedding models get their Bit types; an embedding Bit carries vector and input length", () => {
		expect(bitFor({ id: "qwen-vl", kind: "vision" }).type).toBe(IBitTypes.Vlm);
		const embedding = bitFor(
			{ id: "nomic-embed", kind: "embedding" },
			{ vectorLength: 768 },
		);
		expect(embedding.type).toBe(IBitTypes.Embedding);
		expect(embedding.parameters).toMatchObject({
			vector_length: 768,
			input_length: 16_384,
			pooling: "Mean",
			prefix: { query: "", paragraph: "" },
			provider: {
				provider_name: "device",
				params: { device_id: DEVICE, model: "nomic-embed", kind: "embedding" },
			},
		});
		expect(embedding.parameters.provider.params.api_surface).toBeUndefined();
		expect(() => bitFor({ id: "nomic-embed", kind: "embedding" })).toThrow(
			/nomic-embed needs a vector length/,
		);
	});

	test("a new Bit gets a random id within the hub's id rules, so no one can take it first", () => {
		const id = deviceBitIdFor([], DEVICE, "qwen3-8b-q4");
		expect(id).toMatch(/^[A-Za-z0-9_-]{1,64}$/);
		expect(deviceBitIdFor([], DEVICE, "qwen3-8b-q4")).not.toBe(id);
	});

	test("adding the same model again reuses the user's Bit for that device and model", () => {
		const saved = bitFor(qwen);
		const other = { id: "qwen3-14b", kind: "chat" as const };
		const bits = [
			{ ...bitFor(other), id: "theirs-other-model" },
			{ ...saved, id: "mine" },
		];
		expect(savedDeviceBit(bits, DEVICE, qwen.id)?.id).toBe("mine");
		expect(deviceBitIdFor(bits, DEVICE, qwen.id)).toBe("mine");
		expect(deviceBitIdFor(bits, DEVICE, other.id)).toBe("theirs-other-model");
		expect(deviceBitIdFor(bits, "another-device", qwen.id)).not.toBe("mine");
		const custom = {
			...saved,
			id: "custom-openai",
			parameters: {
				...saved.parameters,
				provider: {
					...saved.parameters.provider,
					provider_name: "custom:openai",
				},
			},
		};
		expect(savedDeviceBit([custom], DEVICE, qwen.id)).toBeUndefined();
	});

	test("the context comes from the model's settings, else a default per kind", () => {
		expect(
			defaultContext({ kind: "chat", settings: { ctx_per_slot: 4096 } }),
		).toBe(4096);
		expect(defaultContext({ kind: "chat", settings: {} })).toBe(8192);
		expect(defaultContext({ kind: "embedding", settings: {} })).toBe(512);
	});

	test("typed numbers are whole, positive and bounded", () => {
		expect(wholeNumber(" 768 ", 1000)).toBe(768);
		expect(wholeNumber("", 1000)).toBeUndefined();
		expect(wholeNumber("0", 1000)).toBeUndefined();
		expect(wholeNumber("12.5", 1000)).toBeUndefined();
		expect(wholeNumber("1001", 1000)).toBeUndefined();
		expect(wholeNumber("abc", 1000)).toBeUndefined();
	});

	test("only a host with the device connector offers the Bit in model pickers", () => {
		const bit = bitFor(qwen);
		const desktop = { canHostLlamaCPP: true, canHostMLX: true };
		expect(isHostableLlmModel(bit, desktop)).toBe(false);
		expect(isHostableLlmModel(bit, { ...desktop, deviceModels: true })).toBe(
			true,
		);
	});
});

describe("saving", () => {
	const refusal = new ApiResponseError({
		status: 400,
		message:
			"Custom bit provider must be 'custom:<provider>' (remote backend), 'Local' (GGUF), or 'MLX'",
	});

	test("the hub's answers map to a typed failure", () => {
		expect(deviceBitFailure(refusal)).toBe("hub_refused");
		expect(
			deviceBitFailure(new ApiResponseError({ status: 403, message: "no" })),
		).toBe("not_allowed");
		expect(
			deviceBitFailure(new ApiResponseError({ status: 503, message: "busy" })),
		).toBe("failed");
		expect(deviceBitFailure(new TypeError("Failed to fetch"))).toBe("failed");
	});

	function backend(
		upsert: IBackendState["bitState"]["upsertCustomBit"],
		addBit: IBackendState["bitState"]["addBit"] = async () => undefined,
	) {
		const calls: string[] = [];
		const state = {
			bitState: {
				upsertCustomBit: async (...args: Parameters<typeof upsert>) => {
					calls.push(`upsert ${args[0].id}`);
					return upsert(...args);
				},
				addBit: async (...args: Parameters<typeof addBit>) => {
					calls.push(`add ${args[0].id} to ${args[1].hub_profile.id}`);
					return addBit(...args);
				},
			},
			userState: {
				getSettingsProfile: async () => ({
					hub_profile: { id: "profile-1", bits: [] },
				}),
			},
		} as unknown as Pick<IBackendState, "bitState" | "userState">;
		return { state, calls };
	}

	test("saves the Bit, then turns it on in the profile in use", async () => {
		const bit = bitFor(qwen);
		const { state, calls } = backend(async (saved) => saved);
		await expect(saveDeviceModelBit(state, bit)).resolves.toEqual({
			bit,
			inProfile: true,
		});
		expect(calls).toEqual([`upsert ${bit.id}`, `add ${bit.id} to profile-1`]);
	});

	test("a profile that doesn't take it keeps the saved Bit and says so", async () => {
		const bit = bitFor(qwen);
		const { state } = backend(
			async (saved) => saved,
			async () => {
				throw new Error("profile write failed");
			},
		);
		await expect(saveDeviceModelBit(state, bit)).resolves.toEqual({
			bit,
			inProfile: false,
		});
	});

	test("a hub before device models refuses it: a typed error, and nothing is added to the profile", async () => {
		const bit = bitFor(qwen);
		const { state, calls } = backend(async () => {
			throw refusal;
		});
		const error = await saveDeviceModelBit(state, bit).catch(
			(caught: unknown) => caught,
		);
		expect(error).toBeInstanceOf(DeviceBitError);
		expect((error as DeviceBitError).code).toBe("hub_refused");
		expect((error as DeviceBitError).message).toContain(
			`Saving device model Bit ${bit.id} failed`,
		);
		expect((error as DeviceBitError).cause).toBe(refusal);
		expect(calls).toEqual([`upsert ${bit.id}`]);
	});
});
