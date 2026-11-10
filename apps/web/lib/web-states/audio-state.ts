import type {
	IAudioState,
	ISpeakRequest,
	ISpeechStream,
	ITranscribeRequest,
	ITranscribeResponse,
} from "@flow-like/flow-like-ui";
import {
	type WebBackendRef,
	apiPost,
	apiPostBlob,
	apiPostStream,
} from "./api-utils";

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

export class WebAudioState implements IAudioState {
	constructor(private readonly backend: WebBackendRef) {}

	async transcribe({
		bit_id,
		audio_base64,
		mime,
		language,
	}: ITranscribeRequest): Promise<ITranscribeResponse> {
		return apiPost<ITranscribeResponse>(
			"audio/transcriptions",
			{ bit_id, audio_base64, mime, language },
			this.backend.auth,
		);
	}

	async speak(request: ISpeakRequest): Promise<Blob> {
		if (request.local) {
			throw new Error("Local speech synthesis requires the desktop app");
		}
		const { bit_id, text, voice, format } = request;
		return apiPostBlob(
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
			throw new Error("Local speech synthesis requires the desktop app");
		}
		const { bit_id, text, voice } = request;
		const response = await apiPostStream(
			"audio/speech",
			{ bit_id, text, voice, format: "pcm", stream: true },
			this.backend.auth,
			options,
		);
		const sampleRate = Number(response.headers.get("x-audio-sample-rate"));
		if (!Number.isInteger(sampleRate) || sampleRate <= 0 || !response.body) {
			await response.body?.cancel().catch(() => {});
			throw new Error(
				`Streamed speech is missing a valid x-audio-sample-rate (got "${response.headers.get("x-audio-sample-rate")}")`,
			);
		}
		return {
			sampleRate,
			chunks: readChunks(response.body.getReader()),
		};
	}
}
