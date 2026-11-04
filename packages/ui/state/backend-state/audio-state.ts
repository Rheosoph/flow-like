export interface ITranscribeRequest {
	bit_id: string;
	/** Base64 audio file bytes (at most 25 MB decoded). */
	audio_base64: string;
	mime: string;
	language?: string;
	/** The bit is an installed local STT model; hosts that can run it on this machine do. */
	local?: boolean;
}

export interface ITranscribeResponse {
	text: string;
	language?: string | null;
	duration_ms?: number | null;
}

export interface ISpeakRequest {
	bit_id: string;
	/** At most 4096 characters. */
	text: string;
	voice?: string;
	/** mp3 (default), wav, opus, aac, flac or pcm. */
	format?: string;
}

export interface IAudioState {
	/** Local STT bits run on this machine (desktop); every other bit goes to the hosted relay. */
	transcribe(request: ITranscribeRequest): Promise<ITranscribeResponse>;
	/** Synthesizes speech through the hosted relay; the blob carries the provider's audio type. */
	speak(request: ISpeakRequest): Promise<Blob>;
}
