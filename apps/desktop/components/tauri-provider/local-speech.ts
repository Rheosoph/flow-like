import type {
	ITranscribeRequest,
	ITranscribeResponse,
} from "@flow-like/flow-like-ui";
import { invoke } from "@tauri-apps/api/core";

function base64ToBytes(base64: string): number[] {
	const binary = atob(base64);
	const bytes = new Array<number>(binary.length);
	for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
	return bytes;
}

/** Runs an installed local STT bit through the desktop's `speech_transcribe` command. */
export async function transcribeLocally(
	request: ITranscribeRequest,
): Promise<ITranscribeResponse> {
	const result = await invoke<{ text: string }>("speech_transcribe", {
		bitId: request.bit_id,
		audio: base64ToBytes(request.audio_base64),
		language: request.language ?? null,
	});
	return { text: result.text };
}
