import { beforeEach, describe, expect, test, vi } from "vitest";
import type { TauriBackend } from "../../tauri-provider";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	post: vi.fn(),
	postBlob: vi.fn(),
	postStream: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../../lib/api", () => ({
	post: mocks.post,
	postBlob: mocks.postBlob,
	postStream: mocks.postStream,
}));

import { AudioState } from "../audio-state";

const profile = { id: "profile" };
const auth = { user: { access_token: "token" } };
const audio = () =>
	new AudioState({ profile, auth } as unknown as TauriBackend);

describe("desktop speech routing", () => {
	beforeEach(() => vi.resetAllMocks());

	test("synthesizes local WAV without requiring a hosted profile or token", async () => {
		const bytes = new Uint8Array([82, 73, 70, 70]);
		mocks.invoke.mockResolvedValue(bytes.buffer);
		const local = new AudioState({} as TauriBackend);
		const blob = await local.speak({
			bit_id: "local-tts",
			text: "Hello",
			voice: "af_heart",
			format: "wav",
			local: true,
		});
		expect(mocks.invoke).toHaveBeenCalledWith("speech_synthesize", {
			bitId: "local-tts",
			text: "Hello",
			voice: "af_heart",
		});
		expect(blob.type).toBe("audio/wav");
		expect(new Uint8Array(await blob.arrayBuffer())).toEqual(bytes);
		expect(mocks.postBlob).not.toHaveBeenCalled();
	});

	test("leaves local voice selection to the model when no voice is set", async () => {
		mocks.invoke.mockResolvedValue(new ArrayBuffer(4));
		await audio().speak({ bit_id: "local-tts", text: "Hello", local: true });
		expect(mocks.invoke).toHaveBeenCalledWith("speech_synthesize", {
			bitId: "local-tts",
			text: "Hello",
			voice: null,
		});
	});

	test("preserves local model failures without falling back to a paid provider", async () => {
		mocks.invoke.mockRejectedValue(new Error("model assets unavailable"));
		await expect(
			audio().speak({
				bit_id: "local-tts",
				text: "Hello",
				local: true,
			}),
		).rejects.toThrow("model assets unavailable");
		expect(mocks.postBlob).not.toHaveBeenCalled();
	});

	test("rejects an unsupported local output format before synthesis", async () => {
		await expect(
			audio().speak({
				bit_id: "local-tts",
				text: "Hello",
				local: true,
				format: "mp3",
			}),
		).rejects.toThrow("WAV output");
		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(mocks.postBlob).not.toHaveBeenCalled();
	});

	test("sends only hosted speech fields to the hosted relay", async () => {
		const blob = new Blob(["audio"], { type: "audio/mpeg" });
		mocks.postBlob.mockResolvedValue(blob);
		expect(
			await audio().speak({
				bit_id: "hosted-tts",
				text: "Hello",
				voice: "nova",
				format: "mp3",
				local: false,
			}),
		).toBe(blob);
		expect(mocks.postBlob).toHaveBeenCalledWith(
			profile,
			"audio/speech",
			{
				bit_id: "hosted-tts",
				text: "Hello",
				voice: "nova",
				format: "mp3",
			},
			auth,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
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
			mocks.postStream.mockResolvedValue(
				streamResponse("24000", [[1, 2], [3]]),
			);
			const signal = new AbortController().signal;
			const stream = await audio().speakStream(
				{ bit_id: "tts", text: "Hello", voice: "nova", format: "mp3" },
				{ signal },
			);
			expect(mocks.postStream).toHaveBeenCalledWith(
				profile,
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
				mocks.postStream.mockResolvedValue(streamResponse(rate, [[1]]));
				await expect(
					audio().speakStream({ bit_id: "tts", text: "Hello" }),
				).rejects.toThrow("x-audio-sample-rate");
			},
		);

		test("lets a failed request reject so callers fall back to speak", async () => {
			mocks.postStream.mockRejectedValue(
				new Error("SPEECH_STREAM_UNSUPPORTED"),
			);
			await expect(
				audio().speakStream({ bit_id: "tts", text: "Hello" }),
			).rejects.toThrow("SPEECH_STREAM_UNSUPPORTED");
		});

		test("never streams local voices", async () => {
			await expect(
				audio().speakStream({ bit_id: "local", text: "Hello", local: true }),
			).rejects.toThrow("cannot be streamed");
			expect(mocks.postStream).not.toHaveBeenCalled();
		});
	});

	describe("warm", () => {
		test("loads only the local models", async () => {
			await audio().warm({
				stt: { bit_id: "stt-local", local: true },
				tts: { bit_id: "tts-hosted", local: false },
			});
			expect(mocks.invoke).toHaveBeenCalledWith("speech_warm", {
				sttBitId: "stt-local",
				ttsBitId: null,
			});
		});

		test("does nothing for hosted models and swallows failures", async () => {
			await audio().warm({ tts: { bit_id: "tts", local: false } });
			expect(mocks.invoke).not.toHaveBeenCalled();
			mocks.invoke.mockRejectedValue(new Error("no model"));
			await expect(
				audio().warm({ stt: { bit_id: "stt", local: true } }),
			).resolves.toBeUndefined();
		});
	});
});
