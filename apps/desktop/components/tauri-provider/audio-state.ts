import type {
	IAudioState,
	ISpeakRequest,
	ITranscribeRequest,
	ITranscribeResponse,
} from "@flow-like/flow-like-ui";
import { post, postBlob } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";
import { transcribeLocally } from "./local-speech";

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
		return postBlob(this.profile(), "audio/speech", request, this.backend.auth);
	}
}
