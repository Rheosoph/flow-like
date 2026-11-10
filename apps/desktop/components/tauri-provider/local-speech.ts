import type {
	ISpeakRequest,
	ISpeechWarmRequest,
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

/** Runs an installed local STT bit through the desktop's `speech_transcribe` commands. */
export async function transcribeLocally(
	request: ITranscribeRequest,
): Promise<ITranscribeResponse> {
	if (request.audio_bytes?.byteLength) {
		const headers: Record<string, string> = { "x-bit-id": request.bit_id };
		if (request.language) headers["x-language"] = request.language;
		const result = await invoke<{ text: string }>(
			"speech_transcribe_raw",
			request.audio_bytes,
			{ headers },
		);
		return { text: result.text };
	}
	const result = await invoke<{ text: string }>("speech_transcribe", {
		bitId: request.bit_id,
		audio: base64ToBytes(request.audio_base64),
		language: request.language ?? null,
	});
	return { text: result.text };
}

/** Loads the installed local speech models ahead of the first utterance; never rejects. */
export async function warmLocally(request: ISpeechWarmRequest): Promise<void> {
	const sttBitId = request.stt?.local ? request.stt.bit_id : null;
	const ttsBitId = request.tts?.local ? request.tts.bit_id : null;
	if (!sttBitId && !ttsBitId) return;
	try {
		await invoke("speech_warm", { sttBitId, ttsBitId });
	} catch (error) {
		console.warn("Local speech warm-up failed", error);
	}
}
