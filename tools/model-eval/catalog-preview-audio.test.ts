import { describe, expect, test } from "bun:test";
import { AUDIO_MODEL_PREVIEWS } from "./catalog-preview-audio";

describe("audio preview attribution", () => {
	test("every catalog target has an exact model guard and a unique assignment", () => {
		const assigned = new Set<string>();
		for (const entry of AUDIO_MODEL_PREVIEWS) {
			expect(entry.model_id).toBeTruthy();
			for (const bitId of entry.bit_ids) {
				expect(assigned.has(bitId)).toBe(false);
				assigned.add(bitId);
			}
			for (const media of entry.media) {
				expect(media.kind).toBe("audio");
				expect(new URL(media.url).protocol).toBe("https:");
				expect(new URL(media.url).pathname).toMatch(/\.(wav|mp3)$/);
				expect(new URL(media.url).search).toBe("");
				expect(new URL(media.source_url).protocol).toBe("https:");
				expect(media.description.length).toBeGreaterThan(30);
			}
		}
	});

	test("the free Fish route keeps its parent-model attribution", () => {
		const free = AUDIO_MODEL_PREVIEWS.find(
			(entry) => entry.model_id === "fish-audio/s2.1-pro-free:free",
		);
		const parent = AUDIO_MODEL_PREVIEWS.find(
			(entry) => entry.model_id === "fish-audio/s2.1-pro",
		);
		expect(free?.media[0].url).toBe(parent?.media[0].url);
		expect(free?.media[0].scope).toBe("base_model_output");
		expect(free?.media[0].description).toContain(
			"free route was not sampled separately",
		);
	});

	test("local Qwen uses the CustomVoice publisher output, not the Base clone input", () => {
		const qwen = AUDIO_MODEL_PREVIEWS.find(
			(entry) => entry.model_id === "Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice",
		);
		expect(qwen?.media[0].url).toEndWith("/Qwen3-TTS-0115/F05.wav");
		expect(qwen?.media[0].description).toContain("CustomVoice");
		expect(qwen?.media[0].description).toContain("Serena");
		expect(qwen?.media[0].scope).toBe("output");
	});

	test("local Kokoro demonstrates the voice installed by the Bit", () => {
		const kokoro = AUDIO_MODEL_PREVIEWS.find(
			(entry) => entry.model_id === "hexgrad/Kokoro-82M",
		);
		expect(kokoro?.media[0].url).toEndWith("/samples/af_heart_0.wav");
		expect(kokoro?.media[0].description).toContain("af_heart");
		expect(kokoro?.media[0].scope).toBe("output");
	});
});
