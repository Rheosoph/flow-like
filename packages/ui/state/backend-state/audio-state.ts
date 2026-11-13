export interface ITranscribeRequest {
	bit_id: string;
	/** Base64 audio file bytes (at most 25 MB decoded). May be empty when `audio_bytes` is set. */
	audio_base64: string;
	/** The same audio file as raw bytes. Hosts that run the model on this machine read these instead. */
	audio_bytes?: Uint8Array;
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
	/** Use an installed local TTS model on desktop. Local synthesis returns WAV. */
	local?: boolean;
}

/** Speech as the provider produces it: signed 16-bit little-endian mono PCM. */
export interface ISpeechStream {
	sampleRate: number;
	chunks: AsyncIterable<Uint8Array>;
}

/** The speech models a call is about to use. */
export interface ISpeechWarmRequest {
	stt?: { bit_id: string; local: boolean };
	tts?: { bit_id: string; local: boolean };
}

export interface IAudioState {
	/** Local STT bits run on this machine (desktop); every other bit goes to the hosted relay. */
	transcribe(request: ITranscribeRequest): Promise<ITranscribeResponse>;
	/** Local TTS bits run on desktop; other bits use the hosted relay. The blob carries the audio type. */
	speak(request: ISpeakRequest): Promise<Blob>;
	/**
	 * Hosted voices only: audio arrives while the provider still synthesizes. Rejects when the voice
	 * or host cannot stream; callers then fall back to `speak`.
	 */
	speakStream?(
		request: ISpeakRequest,
		options?: { signal?: AbortSignal },
	): Promise<ISpeechStream>;
	/** Loads the call's speech models ahead of the first utterance. Failures are not errors. */
	warm?(request: ISpeechWarmRequest): Promise<void>;
}
