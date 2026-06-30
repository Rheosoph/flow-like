"use client";

import { useCallback, useEffect, useRef, useState } from "react";

export interface UseVoiceRecorderOptions {
	/** Maximum recording length in seconds. 0 = unlimited. */
	maxDuration?: number;
	/** Preferred MIME type. Falls back to the platform default when unsupported. */
	mimeType?: string;
	/** Keep recording for this many ms after stop() so trailing words aren't clipped. */
	stopDelay?: number;
	onComplete?: (file: File, durationSeconds: number, blob: Blob) => void;
	onError?: (error: unknown) => void;
}

export interface VoiceRecorder {
	isRecording: boolean;
	/** True between start() and the encoder actually capturing (mic warm-up). */
	isArming: boolean;
	recordingTime: number;
	analyser: AnalyserNode | null;
	isSupported: boolean;
	start: () => Promise<void>;
	stop: () => void;
	cancel: () => void;
	/**
	 * Acquire the mic ahead of time (e.g. on hover) so the next start() captures
	 * instantly on an already-live device. No-ops until mic permission is
	 * granted, so it never triggers a prompt on hover. Auto-releases if unused.
	 */
	prewarm: () => void;
}

const DEFAULT_MIME = "audio/webm";
/** How long a hover-warmed mic stays open before auto-release, if unused. */
const PREWARM_TTL_MS = 6000;

function extensionForMime(mime: string): string {
	if (mime.includes("wav")) return "wav";
	if (mime.includes("ogg")) return "ogg";
	if (mime.includes("mp4") || mime.includes("mpeg")) return "mp4";
	return "webm";
}

/**
 * Encapsulates microphone capture (MediaRecorder), an AnalyserNode for
 * visualizers / silence detection, and an optional max-duration timer.
 * Supports pre-warming the mic so recording can start without the getUserMedia
 * + hardware ramp-up latency. Extracted from the previously duplicated
 * implementations in VoiceInput, VoiceMode and chatbox.
 */
export function useVoiceRecorder({
	maxDuration = 0,
	mimeType = DEFAULT_MIME,
	stopDelay = 0,
	onComplete,
	onError,
}: UseVoiceRecorderOptions = {}): VoiceRecorder {
	const [isRecording, setIsRecording] = useState(false);
	const [isArming, setIsArming] = useState(false);
	const [recordingTime, setRecordingTime] = useState(0);
	const [analyser, setAnalyser] = useState<AnalyserNode | null>(null);

	const recorderRef = useRef<MediaRecorder | null>(null);
	const chunksRef = useRef<Blob[]>([]);
	const streamRef = useRef<MediaStream | null>(null);
	const audioContextRef = useRef<AudioContext | null>(null);
	const acquiringRef = useRef<Promise<MediaStream> | null>(null);
	const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
	const stopTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const warmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
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

	const clearWarmTimer = useCallback(() => {
		if (warmTimerRef.current) {
			clearTimeout(warmTimerRef.current);
			warmTimerRef.current = null;
		}
	}, []);

	const teardown = useCallback(() => {
		if (timerRef.current) {
			clearInterval(timerRef.current);
			timerRef.current = null;
		}
		if (stopTimerRef.current) {
			clearTimeout(stopTimerRef.current);
			stopTimerRef.current = null;
		}
		clearWarmTimer();
		if (streamRef.current) {
			for (const track of streamRef.current.getTracks()) track.stop();
			streamRef.current = null;
		}
		if (audioContextRef.current) {
			audioContextRef.current.close().catch(() => {});
			audioContextRef.current = null;
		}
		setAnalyser(null);
		setIsArming(false);
	}, [clearWarmTimer]);

	// Acquire (or reuse) the mic stream + AnalyserNode. Concurrent calls share
	// one in-flight acquisition so hover-prewarm and a fast click never race.
	const ensureStream = useCallback(async () => {
		if (streamRef.current) return streamRef.current;
		if (acquiringRef.current) return acquiringRef.current;
		const acquisition = (async () => {
			const stream = await navigator.mediaDevices.getUserMedia({
				audio: true,
			});
			streamRef.current = stream;
			const audioContext = new AudioContext();
			// Created after an await, so it can start suspended — resume it or the
			// analyser yields silence and silence/auto-stop detection never fires.
			void audioContext.resume().catch(() => {});
			const source = audioContext.createMediaStreamSource(stream);
			const node = audioContext.createAnalyser();
			node.fftSize = 2048;
			source.connect(node);
			audioContextRef.current = audioContext;
			setAnalyser(node);
			return stream;
		})();
		acquiringRef.current = acquisition;
		try {
			return await acquisition;
		} finally {
			acquiringRef.current = null;
		}
	}, []);

	const prewarm = useCallback(() => {
		if (!isSupported || recorderRef.current) return;
		if (streamRef.current || acquiringRef.current) {
			// already warm/acquiring — just refresh the idle release timer
			clearWarmTimer();
			warmTimerRef.current = setTimeout(teardown, PREWARM_TTL_MS);
			return;
		}
		void (async () => {
			try {
				// Never prompt on hover: only pre-warm once permission is granted.
				const permissions = navigator.permissions;
				if (!permissions?.query) return;
				const status = await permissions.query({
					name: "microphone" as PermissionName,
				});
				if (status.state !== "granted") return;
				if (recorderRef.current || streamRef.current) return;
				await ensureStream();
				clearWarmTimer();
				warmTimerRef.current = setTimeout(teardown, PREWARM_TTL_MS);
			} catch {
				// permission API unsupported or query failed → skip, no prompt
			}
		})();
	}, [isSupported, ensureStream, teardown, clearWarmTimer]);

	const start = useCallback(async () => {
		if (!isSupported || recorderRef.current) return;
		cancelledRef.current = false;
		clearWarmTimer();
		setIsArming(true);
		try {
			const stream = await ensureStream();
			if (cancelledRef.current) {
				teardown();
				return;
			}

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

			// Flip to "recording" only when the encoder is truly capturing, so the
			// indicator never lies during mic warm-up.
			recorder.onstart = () => {
				setIsArming(false);
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

			// Timeslice → the encoder starts promptly and flushes periodically,
			// which avoids dropping the first moments of audio on some browsers.
			recorder.start(250);
		} catch (error) {
			teardown();
			errorRef.current?.(error);
		}
	}, [
		isSupported,
		mimeType,
		maxDuration,
		ensureStream,
		teardown,
		clearWarmTimer,
	]);

	const stop = useCallback(() => {
		const recorder = recorderRef.current;
		if (!recorder || recorder.state !== "recording") return;
		if (stopTimerRef.current) return;
		if (stopDelay > 0) {
			stopTimerRef.current = setTimeout(() => {
				stopTimerRef.current = null;
				if (recorderRef.current?.state === "recording") {
					recorderRef.current.stop();
				}
			}, stopDelay);
		} else {
			recorder.stop();
		}
	}, [stopDelay]);

	const cancel = useCallback(() => {
		cancelledRef.current = true;
		if (stopTimerRef.current) {
			clearTimeout(stopTimerRef.current);
			stopTimerRef.current = null;
		}
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
		isArming,
		recordingTime,
		analyser,
		isSupported,
		start,
		stop,
		cancel,
		prewarm,
	};
}
