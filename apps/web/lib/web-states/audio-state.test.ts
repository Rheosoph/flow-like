import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	apiPost: vi.fn(),
	apiPostBlob: vi.fn(),
	apiPostStream: vi.fn(),
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

	describe("streamed speech", () => {
		const streamResponse = (rate: string | null, parts: number[][]) =>
			new Response(
				new ReadableStream<Uint8Array>({
					start(controller) {
						for (const part of parts) controller.enqueue(new Uint8Array(part));
						controller.close();
					},
				}),
				{ headers: rate === null ? {} : { "x-audio-sample-rate": rate } },
			);

		test("requests raw PCM and iterates the body chunks", async () => {
			mocks.apiPostStream.mockResolvedValue(
				streamResponse("24000", [[1, 2], [3]]),
			);
			const signal = new AbortController().signal;
			const stream = await audio().speakStream(
				{ bit_id: "tts", text: "Hello", voice: "nova", format: "mp3" },
				{ signal },
			);
			expect(mocks.apiPostStream).toHaveBeenCalledWith(
				"audio/speech",
				{
					bit_id: "tts",
					text: "Hello",
					voice: "nova",
					format: "pcm",
					stream: true,
				},
				auth,
				{ signal },
			);
			expect(stream.sampleRate).toBe(24000);
			const received: number[][] = [];
			for await (const chunk of stream.chunks) received.push([...chunk]);
			expect(received).toEqual([[1, 2], [3]]);
		});

		test.each([null, "abc", "0", "-5", "24000.5"])(
			"rejects a sample rate header of %s",
			async (rate) => {
				mocks.apiPostStream.mockResolvedValue(streamResponse(rate, [[1]]));
				await expect(
					audio().speakStream({ bit_id: "tts", text: "Hello" }),
				).rejects.toThrow("x-audio-sample-rate");
			},
		);

		test("lets a failed request reject so callers fall back to speak", async () => {
			mocks.apiPostStream.mockRejectedValue(
				new Error("SPEECH_STREAM_UNSUPPORTED"),
			);
			await expect(
				audio().speakStream({ bit_id: "tts", text: "Hello" }),
			).rejects.toThrow("SPEECH_STREAM_UNSUPPORTED");
		});

		test("never streams local voices", async () => {
			await expect(
				audio().speakStream({ bit_id: "local", text: "Hello", local: true }),
			).rejects.toThrow("requires the desktop app");
			expect(mocks.apiPostStream).not.toHaveBeenCalled();
		});
	});
});
