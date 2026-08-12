"use client";

import { useCallback, useEffect, useRef, useState } from "react";

// The Web Speech API is not in the standard TS DOM lib; model it loosely.
interface SpeechAlternativeLike {
	transcript: string;
}
interface SpeechResultLike extends ArrayLike<SpeechAlternativeLike> {
	isFinal: boolean;
}
interface SpeechRecognitionEventLike {
	resultIndex: number;
	results: ArrayLike<SpeechResultLike>;
}
type SpeechRecognitionLike = {
	continuous: boolean;
	interimResults: boolean;
	lang: string;
	start: () => void;
	stop: () => void;
	abort?: () => void;
	onresult: ((event: SpeechRecognitionEventLike) => void) | null;
	onerror: ((error: unknown) => void) | null;
	onend: (() => void) | null;
};

interface SpeechRecognitionWindow {
	SpeechRecognition?: new () => SpeechRecognitionLike;
	webkitSpeechRecognition?: new () => SpeechRecognitionLike;
}

export interface UseSpeechRecognitionOptions {
	lang?: string;
	continuous?: boolean;
	interimResults?: boolean;
	onResult?: (finalText: string, interimText: string) => void;
	onEnd?: (finalText: string) => void;
	onError?: (error: unknown) => void;
}

export interface SpeechRecognitionState {
	isSupported: boolean;
	isListening: boolean;
	transcript: string;
	start: () => void;
	stop: () => void;
	/** Discard the active session without delivering onEnd. */
	cancel: () => void;
	reset: () => void;
}

function getRecognitionCtor(): (new () => SpeechRecognitionLike) | null {
	if (typeof window === "undefined") return null;
	const w = window as unknown as SpeechRecognitionWindow;
	return w.SpeechRecognition || w.webkitSpeechRecognition || null;
}

/**
 * Platform speech-to-text via the browser Web Speech API. Exposes `isSupported`
 * so callers can fall back to recording when unavailable. Extracted from the
 * inline transcription logic in chatbox.
 */
export function useSpeechRecognition({
	lang,
	continuous = true,
	interimResults = true,
	onResult,
	onEnd,
	onError,
}: UseSpeechRecognitionOptions = {}): SpeechRecognitionState {
	const [isListening, setIsListening] = useState(false);
	const [transcript, setTranscript] = useState("");
	const recognitionRef = useRef<SpeechRecognitionLike | null>(null);
	const stoppingRef = useRef(false);
	const finalRef = useRef("");
	const transcriptRef = useRef("");

	const resultRef = useRef(onResult);
	const endRef = useRef(onEnd);
	const errorRef = useRef(onError);
	resultRef.current = onResult;
	endRef.current = onEnd;
	errorRef.current = onError;

	const isSupported = getRecognitionCtor() !== null;

	const stop = useCallback(() => {
		const recognition = recognitionRef.current;
		if (recognition && !stoppingRef.current) {
			stoppingRef.current = true;
			try {
				recognition.stop();
			} catch {
				recognitionRef.current = null;
				stoppingRef.current = false;
				setIsListening(false);
			}
		}
	}, []);

	const cancel = useCallback(() => {
		const recognition = recognitionRef.current;
		recognitionRef.current = null;
		stoppingRef.current = false;
		if (recognition) {
			recognition.onresult = null;
			recognition.onerror = null;
			recognition.onend = null;
			try {
				if (recognition.abort) recognition.abort();
				else recognition.stop();
			} catch {}
		}
		setIsListening(false);
	}, []);

	const reset = useCallback(() => {
		finalRef.current = "";
		transcriptRef.current = "";
		setTranscript("");
	}, []);

	const start = useCallback(() => {
		const Ctor = getRecognitionCtor();
		if (!Ctor || recognitionRef.current) return;

		let recognition: SpeechRecognitionLike;
		try {
			recognition = new Ctor();
			recognition.continuous = continuous;
			recognition.interimResults = interimResults;
			recognition.lang =
				lang ||
				(typeof navigator !== "undefined" ? navigator.language : "en-US") ||
				"en-US";
		} catch (error) {
			stoppingRef.current = false;
			setIsListening(false);
			errorRef.current?.(error);
			return;
		}
		let failed = false;
		stoppingRef.current = false;

		recognition.onresult = (event: SpeechRecognitionEventLike) => {
			let interim = "";
			for (let i = event.resultIndex; i < event.results.length; i++) {
				const result = event.results[i];
				if (result.isFinal) {
					finalRef.current =
						`${finalRef.current} ${result[0].transcript}`.trim();
				} else interim += result[0].transcript;
			}
			const finalText = finalRef.current.trim();
			const interimText = interim.trim();
			const currentTranscript = interimText
				? `${finalText} ${interimText}`.trim()
				: finalText;
			transcriptRef.current = currentTranscript;
			setTranscript(currentTranscript);
			resultRef.current?.(finalText, interimText);
		};
		recognition.onerror = (error) => {
			failed = true;
			stoppingRef.current = false;
			if (recognitionRef.current === recognition) {
				recognitionRef.current = null;
			}
			setIsListening(false);
			errorRef.current?.(error);
			recognition.onresult = null;
			recognition.onerror = null;
			recognition.onend = null;
		};
		recognition.onend = () => {
			stoppingRef.current = false;
			setIsListening(false);
			if (recognitionRef.current === recognition) {
				recognitionRef.current = null;
			}
			if (!failed) endRef.current?.(transcriptRef.current.trim());
		};

		recognitionRef.current = recognition;
		try {
			recognition.start();
			setIsListening(true);
		} catch (error) {
			recognitionRef.current = null;
			stoppingRef.current = false;
			setIsListening(false);
			errorRef.current?.(error);
			recognition.onresult = null;
			recognition.onerror = null;
			recognition.onend = null;
		}
	}, [continuous, interimResults, lang]);

	useEffect(
		() => () => {
			const recognition = recognitionRef.current;
			recognitionRef.current = null;
			if (recognition) {
				// Web Speech may dispatch end/results asynchronously after stop(). Detach
				// callbacks first so unmounting cannot submit text or update stale state.
				recognition.onresult = null;
				recognition.onerror = null;
				recognition.onend = null;
				try {
					recognition.stop();
				} catch {}
			}
		},
		[],
	);

	return { isSupported, isListening, transcript, start, stop, cancel, reset };
}
