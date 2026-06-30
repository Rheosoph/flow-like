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
	onresult: ((event: SpeechRecognitionEventLike) => void) | null;
	onerror: (() => void) | null;
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
}

export interface SpeechRecognitionState {
	isSupported: boolean;
	isListening: boolean;
	transcript: string;
	start: () => void;
	stop: () => void;
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
}: UseSpeechRecognitionOptions = {}): SpeechRecognitionState {
	const [isListening, setIsListening] = useState(false);
	const [transcript, setTranscript] = useState("");
	const recognitionRef = useRef<SpeechRecognitionLike | null>(null);
	const finalRef = useRef("");

	const resultRef = useRef(onResult);
	const endRef = useRef(onEnd);
	resultRef.current = onResult;
	endRef.current = onEnd;

	const isSupported = getRecognitionCtor() !== null;

	const stop = useCallback(() => {
		if (recognitionRef.current) {
			recognitionRef.current.stop();
			recognitionRef.current = null;
		}
		setIsListening(false);
	}, []);

	const reset = useCallback(() => {
		finalRef.current = "";
		setTranscript("");
	}, []);

	const start = useCallback(() => {
		const Ctor = getRecognitionCtor();
		if (!Ctor || recognitionRef.current) return;

		const recognition = new Ctor();
		recognition.continuous = continuous;
		recognition.interimResults = interimResults;
		recognition.lang =
			lang ||
			(typeof navigator !== "undefined" ? navigator.language : "en-US") ||
			"en-US";

		recognition.onresult = (event: SpeechRecognitionEventLike) => {
			let interim = "";
			for (let i = event.resultIndex; i < event.results.length; i++) {
				const result = event.results[i];
				if (result.isFinal) finalRef.current += result[0].transcript;
				else interim += result[0].transcript;
			}
			const finalText = finalRef.current.trim();
			const interimText = interim.trim();
			setTranscript(
				interimText ? `${finalText} ${interimText}`.trim() : finalText,
			);
			resultRef.current?.(finalText, interimText);
		};
		recognition.onerror = () => setIsListening(false);
		recognition.onend = () => {
			setIsListening(false);
			recognitionRef.current = null;
			endRef.current?.(finalRef.current.trim());
		};

		recognitionRef.current = recognition;
		recognition.start();
		setIsListening(true);
	}, [continuous, interimResults, lang]);

	useEffect(
		() => () => {
			if (recognitionRef.current) recognitionRef.current.stop();
		},
		[],
	);

	return { isSupported, isListening, transcript, start, stop, reset };
}
