"use client";

import { useCallback, useEffect, useRef, useState } from "react";

export interface AnswerPlayback {
	stop: () => void;
	/** Analyser over the playing answer audio (best-effort; null when unavailable). */
	analyser: AnalyserNode | null;
	isPlaying: boolean;
}

/**
 * Autoplays the assistant's most recent audio answer (when playback is enabled),
 * exposes `stop()` to interrupt it, and best-effort exposes an AnalyserNode +
 * `isPlaying` so a visualizer can react to the spoken answer. The Web Audio
 * graph is guarded — if it can't be built, the audio still plays normally and
 * the analyser is simply null. Browser autoplay restrictions are tolerated.
 */
export function useAnswerPlayback(
	enabled: boolean,
	latestAudioUrl: string | null,
): AnswerPlayback {
	const audioRef = useRef<HTMLAudioElement | null>(null);
	const ctxRef = useRef<AudioContext | null>(null);
	const lastUrlRef = useRef<string | null>(null);
	const [analyser, setAnalyser] = useState<AnalyserNode | null>(null);
	const [isPlaying, setIsPlaying] = useState(false);

	const stop = useCallback(() => {
		if (audioRef.current) {
			audioRef.current.pause();
			audioRef.current = null;
		}
		setIsPlaying(false);
	}, []);

	useEffect(() => {
		if (!enabled || !latestAudioUrl) return;
		if (lastUrlRef.current === latestAudioUrl) return;
		lastUrlRef.current = latestAudioUrl;
		if (audioRef.current) audioRef.current.pause();

		const audio = new Audio(latestAudioUrl);
		audioRef.current = audio;

		let source: MediaElementAudioSourceNode | null = null;
		let node: AnalyserNode | null = null;
		try {
			if (!ctxRef.current) ctxRef.current = new AudioContext();
			const ctx = ctxRef.current;
			void ctx.resume().catch(() => {});
			source = ctx.createMediaElementSource(audio);
			node = ctx.createAnalyser();
			node.fftSize = 2048;
			source.connect(node);
			node.connect(ctx.destination);
			setAnalyser(node);
		} catch {
			// Web Audio unavailable / cross-origin: play directly, no analyser.
			setAnalyser(null);
		}

		const onPlay = () => setIsPlaying(true);
		const onStop = () => setIsPlaying(false);
		audio.addEventListener("playing", onPlay);
		audio.addEventListener("ended", onStop);
		audio.addEventListener("pause", onStop);
		audio.play().catch(() => {});

		return () => {
			audio.removeEventListener("playing", onPlay);
			audio.removeEventListener("ended", onStop);
			audio.removeEventListener("pause", onStop);
			audio.pause();
			// Disconnect the per-URL graph nodes so they don't accumulate on the
			// reused AudioContext across answers.
			source?.disconnect();
			node?.disconnect();
		};
	}, [enabled, latestAudioUrl]);

	useEffect(
		() => () => {
			stop();
			if (ctxRef.current) {
				ctxRef.current.close().catch(() => {});
				ctxRef.current = null;
			}
		},
		[stop],
	);

	return { stop, analyser, isPlaying };
}
