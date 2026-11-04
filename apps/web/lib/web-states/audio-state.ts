import type {
	IAudioState,
	ISpeakRequest,
	ITranscribeRequest,
	ITranscribeResponse,
} from "@flow-like/flow-like-ui";
import { type WebBackendRef, apiPost, apiPostBlob } from "./api-utils";

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
		return apiPostBlob("audio/speech", request, this.backend.auth);
	}
}
