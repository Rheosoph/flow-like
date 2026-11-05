import { beforeEach, describe, expect, test, vi } from "vitest";
import type { TauriBackend } from "../../tauri-provider";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	post: vi.fn(),
	postBlob: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../../lib/api", () => ({
	post: mocks.post,
	postBlob: mocks.postBlob,
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
});
