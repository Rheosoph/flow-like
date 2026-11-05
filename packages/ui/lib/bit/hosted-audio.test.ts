import { expect, test } from "bun:test";
import { validateBitDraft } from "../../components/bits/bit-editor-model";
import { type IBit, IBitTypes } from "../schema/bit/bit";
import {
	HOSTED_AUDIO_PROVIDERS,
	buildHostedAudioBit,
	createHostedAudioParameters,
	isHostedAudioProvider,
	updateHostedAudioBit,
	validateHostedAudioParameters,
} from "./hosted-audio";

function draft(type = IBitTypes.Tts): IBit {
	return {
		id: "audio",
		type,
		authors: [],
		dependencies: ["weights"],
		dependency_tree_hash: "weights-hash",
		hash: "",
		hub: "hub",
		created: "",
		updated: "",
		download_link: "https://example.test/weights",
		file_name: "weights",
		size: 100,
		parameters: { assets: ["weights"], runtime: "Auto" },
		meta: {
			en: {
				name: "Speech",
				description: "",
				tags: [],
				preview_media: [],
				created_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
				updated_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
			},
		},
	};
}
function parameters(
	provider_name = "hosted:openrouter",
	model_id = "provider/audio-model",
) {
	return { provider: { provider_name, model_id, params: { tier: "FREE" } } };
}

test("hosted audio registration clears every local asset field for both speech tasks", () => {
	for (const type of [IBitTypes.Stt, IBitTypes.Tts]) {
		const local = draft(type);
		const hosted = updateHostedAudioBit(local, {
			...local.parameters,
			...parameters(),
		});
		const registered = buildHostedAudioBit(hosted);
		expect(registered.type).toBe(type);
		expect(registered.file_name).toBeNull();
		expect(registered.download_link).toBeNull();
		expect(registered.dependencies).toEqual([]);
		expect(registered.dependency_tree_hash).toBe("");
		expect(registered.size).toBe(0);
		expect(registered.parameters).toEqual(parameters());
		expect(registered.meta).toEqual(local.meta);
		expect(local.dependencies).toEqual(["weights"]);
		expect(validateBitDraft(registered, "admin")).toBeNull();
		expect(validateBitDraft(registered, "custom")).toContain("administrator");
	}
});

test("audio configuration accepts exactly the gateway providers and requires an explicit model", () => {
	for (const provider of HOSTED_AUDIO_PROVIDERS) {
		expect(
			validateHostedAudioParameters(parameters(provider.value)),
		).toBeNull();
		expect(isHostedAudioProvider(` ${provider.value.toUpperCase()} `)).toBe(
			true,
		);
	}
	for (const name of [
		"Hosted",
		"Premium",
		"Internal",
		"hosted:anthropic",
		"hosted:vertex",
		"custom:openrouter",
		"Local",
		null,
	]) {
		expect(isHostedAudioProvider(name)).toBe(false);
		if (name)
			expect(validateHostedAudioParameters(parameters(name))).toContain(
				"provider",
			);
	}
	for (const model of [
		"",
		" ",
		"a//b",
		"../model",
		"a/../b",
		"a?key=x",
		"a".repeat(257),
	]) {
		expect(
			validateHostedAudioParameters(parameters("hosted:openai", model)),
		).toContain("model ID");
	}
	expect(createHostedAudioParameters()).toEqual({
		provider: { provider_name: "hosted:openrouter", model_id: "" },
	});
	expect(() =>
		buildHostedAudioBit(
			updateHostedAudioBit(draft(), createHostedAudioParameters()),
		),
	).toThrow("model ID");
});

test("audio edits reject credentials, API overrides and local files before saving", () => {
	for (const params of [
		{ endpoint: "https://example.test" },
		{ api_key: "secret" },
		{ tier: "" },
		{ tier: 3 },
		[],
	]) {
		expect(
			validateHostedAudioParameters({
				provider: { ...parameters().provider, params },
			}),
		).not.toBeNull();
	}
	expect(
		validateHostedAudioParameters({
			provider: { ...parameters().provider, api_surface: "ChatCompletions" },
		}),
	).toContain("API surface");
	const valid = updateHostedAudioBit(draft(), parameters());
	for (const change of [
		{ file_name: "" },
		{ download_link: "" },
		{ dependencies: ["weights"] },
		{ size: 1 },
		{ parameters: { ...parameters(), assets: ["weights"] } },
	]) {
		expect(validateBitDraft({ ...valid, ...change }, "admin")).toContain(
			"local files",
		);
	}
});

test("speech defaults survive registration and edits without retaining local settings", () => {
	const defaults = {
		...parameters(),
		default_voice: "af_heart",
		default_language: "pt-BR",
	};
	const voice = buildHostedAudioBit(updateHostedAudioBit(draft(), defaults));
	expect(voice.parameters).toEqual({
		...parameters(),
		default_voice: "af_heart",
	});
	const transcription = buildHostedAudioBit(
		updateHostedAudioBit(draft(IBitTypes.Stt), defaults),
	);
	expect(transcription.parameters).toEqual({
		...parameters(),
		default_language: "pt-BR",
	});
	for (const key of ["default_voice", "default_language"]) {
		for (const value of ["", "space in id", "a".repeat(65), 4, []]) {
			expect(
				validateHostedAudioParameters({ ...parameters(), [key]: value }),
			).not.toBeNull();
		}
		expect(
			validateHostedAudioParameters({ ...parameters(), [key]: null }),
		).toBeNull();
	}
});

test("editing a hosted audio Bit preserves its published model tariff", () => {
	for (const [type, pricing] of [
		[IBitTypes.Tts, { input_micro_usd_per_million_characters: 80_000_000 }],
		[IBitTypes.Tts, { input_micro_usd_per_million_bytes: 15_000_000 }],
		[IBitTypes.Stt, { input_micro_usd_per_million_seconds: 100_000_000 }],
	] as const) {
		const configured = { ...parameters(), pricing };
		const edited = buildHostedAudioBit(
			updateHostedAudioBit(draft(type), configured),
		);
		expect(edited.parameters).toEqual(configured);
	}
	const bounded = { ...parameters(), max_input_characters: 2000 };
	expect(
		buildHostedAudioBit(updateHostedAudioBit(draft(), bounded)).parameters,
	).toEqual(bounded);
});
