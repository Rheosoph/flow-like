import type {
	ISpeakRequest,
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

/** Synthesizes WAV audio with the desktop's installed local TTS model. */
export async function speakLocally(request: ISpeakRequest): Promise<Blob> {
	if (request.format && request.format !== "wav") {
		throw new Error("Local speech synthesis supports WAV output");
	}
	const wav = await invoke<ArrayBuffer>("speech_synthesize", {
		bitId: request.bit_id,
		text: request.text,
		voice: request.voice ?? null,
	});
	return new Blob([wav], { type: "audio/wav" });
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
