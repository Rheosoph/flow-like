import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	apiPost: vi.fn(),
	apiPostBlob: vi.fn(),
}));
vi.mock("./api-utils", () => mocks);

import { WebAudioState } from "./audio-state";

const auth = { user: { access_token: "token" } };
const audio = () => new WebAudioState({ auth } as never);

describe("hosted audio", () => {
	beforeEach(() => vi.resetAllMocks());

	test("transcribes with the catalog bit and base64 audio only", async () => {
		mocks.apiPost.mockResolvedValue({ text: "hello", duration_ms: 1200 });
		const result = await audio().transcribe({
			bit_id: "stt-bit",
			audio_base64: "UklGRg==",
			mime: "audio/wav",
			language: "en",
			local: false,
		});
		expect(mocks.apiPost).toHaveBeenCalledWith(
			"audio/transcriptions",
			{
				bit_id: "stt-bit",
				audio_base64: "UklGRg==",
				mime: "audio/wav",
				language: "en",
			},
			auth,
		);
		expect(result.text).toBe("hello");
	});

	test("speaks through the speech route and returns the audio blob", async () => {
		const blob = new Blob(["audio"], { type: "audio/mpeg" });
		mocks.apiPostBlob.mockResolvedValue(blob);
		const result = await audio().speak({
			bit_id: "tts-bit",
			text: "Hello",
			voice: "nova",
			format: "mp3",
			local: false,
		});
		expect(mocks.apiPostBlob).toHaveBeenCalledWith(
			"audio/speech",
			{ bit_id: "tts-bit", text: "Hello", voice: "nova", format: "mp3" },
			auth,
		);
		expect(result).toBe(blob);
	});

	test("refuses local synthesis without sending a hosted request", async () => {
		await expect(
			audio().speak({ bit_id: "local-tts", text: "Hello", local: true }),
		).rejects.toThrow("requires the desktop app");
		expect(mocks.apiPostBlob).not.toHaveBeenCalled();
	});
});
