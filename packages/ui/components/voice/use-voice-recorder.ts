"use client";

import { useCallback, useEffect, useRef, useState } from "react";

export interface UseVoiceRecorderOptions {
	/** Maximum recording length in seconds. 0 = unlimited. */
	maxDuration?: number;
	/** Preferred MIME type. Falls back to the platform default when unsupported. */
	mimeType?: string;
	onComplete?: (file: File, durationSeconds: number, blob: Blob) => void;
	onError?: (error: unknown) => void;
}

export interface VoiceRecorder {
	isRecording: boolean;
	recordingTime: number;
	analyser: AnalyserNode | null;
	isSupported: boolean;
	start: () => Promise<void>;
	stop: () => void;
	cancel: () => void;
}

const DEFAULT_MIME = "audio/webm";

function extensionForMime(mime: string): string {
	if (mime.includes("wav")) return "wav";
	if (mime.includes("ogg")) return "ogg";
	if (mime.includes("mp4") || mime.includes("mpeg")) return "mp4";
	return "webm";
}

/**
 * Encapsulates microphone capture (MediaRecorder), an AnalyserNode for
 * visualizers / silence detection, and an optional max-duration timer.
 * Extracted from the previously duplicated implementations in VoiceInput,
 * VoiceMode and chatbox.
 */
export function useVoiceRecorder({
	maxDuration = 0,
	mimeType = DEFAULT_MIME,
	onComplete,
	onError,
}: UseVoiceRecorderOptions = {}): VoiceRecorder {
	const [isRecording, setIsRecording] = useState(false);
	const [recordingTime, setRecordingTime] = useState(0);
	const [analyser, setAnalyser] = useState<AnalyserNode | null>(null);

	const recorderRef = useRef<MediaRecorder | null>(null);
	const chunksRef = useRef<Blob[]>([]);
	const streamRef = useRef<MediaStream | null>(null);
	const audioContextRef = useRef<AudioContext | null>(null);
	const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
	const timeRef = useRef(0);
	const cancelledRef = useRef(false);

	const completeRef = useRef(onComplete);
	const errorRef = useRef(onError);
	completeRef.current = onComplete;
	errorRef.current = onError;

	const isSupported =
		typeof navigator !== "undefined" &&
		typeof navigator.mediaDevices?.getUserMedia === "function" &&
		typeof window !== "undefined" &&
		typeof window.MediaRecorder !== "undefined";

	const teardown = useCallback(() => {
		if (timerRef.current) {
			clearInterval(timerRef.current);
			timerRef.current = null;
		}
		if (streamRef.current) {
			for (const track of streamRef.current.getTracks()) track.stop();
			streamRef.current = null;
		}
		if (audioContextRef.current) {
			audioContextRef.current.close().catch(() => {});
			audioContextRef.current = null;
		}
		setAnalyser(null);
	}, []);

	const start = useCallback(async () => {
		if (!isSupported || recorderRef.current) return;
		cancelledRef.current = false;
		try {
			const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
			streamRef.current = stream;

			const audioContext = new AudioContext();
			const source = audioContext.createMediaStreamSource(stream);
			const node = audioContext.createAnalyser();
			node.fftSize = 2048;
			source.connect(node);
			audioContextRef.current = audioContext;
			setAnalyser(node);

			const useMime =
				typeof window.MediaRecorder.isTypeSupported === "function" &&
				window.MediaRecorder.isTypeSupported(mimeType)
					? mimeType
					: undefined;
			const recorder = useMime
				? new MediaRecorder(stream, { mimeType: useMime })
				: new MediaRecorder(stream);
			recorderRef.current = recorder;
			chunksRef.current = [];

			recorder.ondataavailable = (event) => {
				if (event.data.size > 0) chunksRef.current.push(event.data);
			};

			recorder.onstop = () => {
				const finalDuration = timeRef.current;
				const cancelled = cancelledRef.current;
				recorderRef.current = null;
				teardown();
				setIsRecording(false);

				const type = recorder.mimeType || mimeType;
				const chunks = chunksRef.current;
				chunksRef.current = [];
				if (cancelled) return;

				const blob = new Blob(chunks, { type });
				if (blob.size === 0) return;
				const ext = extensionForMime(type);
				const file = new File([blob], `voice-${Date.now()}.${ext}`, { type });
				completeRef.current?.(file, finalDuration, blob);
			};

			recorder.start();
			setIsRecording(true);
			setRecordingTime(0);
			timeRef.current = 0;

			timerRef.current = setInterval(() => {
				timeRef.current += 1;
				setRecordingTime(timeRef.current);
				if (maxDuration > 0 && timeRef.current >= maxDuration) {
					recorderRef.current?.stop();
				}
			}, 1000);
		} catch (error) {
			teardown();
			errorRef.current?.(error);
		}
	}, [isSupported, mimeType, maxDuration, teardown]);

	const stop = useCallback(() => {
		const recorder = recorderRef.current;
		if (recorder && recorder.state === "recording") recorder.stop();
	}, []);

	const cancel = useCallback(() => {
		cancelledRef.current = true;
		const recorder = recorderRef.current;
		if (recorder && recorder.state === "recording") {
			recorder.stop();
		} else {
			teardown();
			setIsRecording(false);
		}
	}, [teardown]);

	useEffect(
		() => () => {
			cancelledRef.current = true;
			if (recorderRef.current && recorderRef.current.state === "recording") {
				recorderRef.current.stop();
			}
			teardown();
		},
		[teardown],
	);

	return {
		isRecording,
		recordingTime,
		analyser,
		isSupported,
		start,
		stop,
		cancel,
	};
}
