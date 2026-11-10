import type {
	IAudioState,
	ISpeakRequest,
	ISpeechStream,
	ISpeechWarmRequest,
	ITranscribeRequest,
	ITranscribeResponse,
} from "@flow-like/flow-like-ui";
import { post, postBlob, postStream } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";
import { speakLocally, transcribeLocally, warmLocally } from "./local-speech";

async function* readChunks(
	reader: ReadableStreamDefaultReader<Uint8Array>,
): AsyncGenerator<Uint8Array> {
	try {
		while (true) {
			const { done, value } = await reader.read();
			if (done) return;
			if (value?.byteLength) yield value;
		}
	} finally {
		await reader.cancel().catch(() => {});
	}
}

export class AudioState implements IAudioState {
	constructor(private readonly backend: TauriBackend) {}

	private profile(): NonNullable<TauriBackend["profile"]> {
		const profile = this.backend.profile;
		if (!profile) throw new Error("Profile context is not available");
		return profile;
	}

	async transcribe(request: ITranscribeRequest): Promise<ITranscribeResponse> {
		if (request.local) return transcribeLocally(request);
		const { bit_id, audio_base64, mime, language } = request;
		return post<ITranscribeResponse>(
			this.profile(),
			"audio/transcriptions",
			{ bit_id, audio_base64, mime, language },
			this.backend.auth,
		);
	}

	async speak(request: ISpeakRequest): Promise<Blob> {
		if (request.local) return speakLocally(request);
		const { bit_id, text, voice, format } = request;
		return postBlob(
			this.profile(),
			"audio/speech",
			{ bit_id, text, voice, format },
			this.backend.auth,
		);
	}

	async speakStream(
		request: ISpeakRequest,
		options?: { signal?: AbortSignal },
	): Promise<ISpeechStream> {
		if (request.local) {
			throw new Error("Local speech synthesis cannot be streamed");
		}
		const { bit_id, text, voice } = request;
		const response = await postStream(
			this.profile(),
			"audio/speech",
			{ bit_id, text, voice, format: "pcm", stream: true },
			this.backend.auth,
			options,
		);
		const header = response.headers.get("x-audio-sample-rate");
		const sampleRate = Number(header);
		if (!Number.isInteger(sampleRate) || sampleRate <= 0 || !response.body) {
			await response.body?.cancel().catch(() => {});
			throw new Error(
				`Streamed speech is missing a valid x-audio-sample-rate (got "${header}")`,
			);
		}
		return {
			sampleRate,
			chunks: readChunks(response.body.getReader()),
		};
	}

	warm(request: ISpeechWarmRequest): Promise<void> {
		return warmLocally(request);
	}
}
