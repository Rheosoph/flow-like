"use client";

import { useCallback, useEffect, useRef, useState } from "react";

export interface AudioPlaybackState {
	isPlaying: boolean;
	currentTime: number;
	duration: number;
	/** Analyser over the playing audio (best-effort; null on cross-origin sources). */
	analyser: AnalyserNode | null;
	play: () => void;
	pause: () => void;
	toggle: () => void;
	seek: (seconds: number) => void;
}

/**
 * User-driven playback of a single audio source with a best-effort AnalyserNode
 * so the voice visualizers can react to the waveform. Sibling to
 * useVoiceRecorder and useAnswerPlayback — the latter autoplays the newest
 * answer, this one is interactive (play/pause/seek). The Web Audio graph is
 * guarded: if it can't be built (e.g. cross-origin without CORS) the audio
 * still plays and `analyser` is simply null.
 */
export function useAudioPlayback(
	src: string | null | undefined,
	autoPlay = false,
): AudioPlaybackState {
	const audioRef = useRef<HTMLAudioElement | null>(null);
	const ctxRef = useRef<AudioContext | null>(null);
	const sourceRef = useRef<MediaElementAudioSourceNode | null>(null);

	const [analyser, setAnalyser] = useState<AnalyserNode | null>(null);
	const [isPlaying, setIsPlaying] = useState(false);
	const [currentTime, setCurrentTime] = useState(0);
	const [duration, setDuration] = useState(0);

	const ensureGraph = useCallback(() => {
		const audio = audioRef.current;
		if (!audio || sourceRef.current) return;
		try {
			const ctx = ctxRef.current ?? new AudioContext();
			ctxRef.current = ctx;
			const source = ctx.createMediaElementSource(audio);
			const node = ctx.createAnalyser();
			node.fftSize = 2048;
			source.connect(node);
			node.connect(ctx.destination);
			sourceRef.current = source;
			setAnalyser(node);
		} catch {
			setAnalyser(null);
		}
	}, []);

	useEffect(() => {
		setIsPlaying(false);
		setCurrentTime(0);
		setDuration(0);
		setAnalyser(null);
		sourceRef.current = null;

		if (!src) {
			if (audioRef.current) {
				audioRef.current.pause();
				audioRef.current = null;
			}
			return;
		}

		const audio = new Audio(src);
		audio.preload = "metadata";
		audioRef.current = audio;

		const onTime = () => setCurrentTime(audio.currentTime);
		const onMeta = () =>
			setDuration(Number.isFinite(audio.duration) ? audio.duration : 0);
		const onPlay = () => setIsPlaying(true);
		const onPause = () => setIsPlaying(false);
		const onEnded = () => {
			setIsPlaying(false);
			setCurrentTime(0);
		};
		audio.addEventListener("timeupdate", onTime);
		audio.addEventListener("loadedmetadata", onMeta);
		audio.addEventListener("durationchange", onMeta);
		audio.addEventListener("play", onPlay);
		audio.addEventListener("pause", onPause);
		audio.addEventListener("ended", onEnded);

		if (autoPlay) {
			ensureGraph();
			void ctxRef.current?.resume().catch(() => {});
			void audio.play().catch(() => {});
		}

		return () => {
			audio.removeEventListener("timeupdate", onTime);
			audio.removeEventListener("loadedmetadata", onMeta);
			audio.removeEventListener("durationchange", onMeta);
			audio.removeEventListener("play", onPlay);
			audio.removeEventListener("pause", onPause);
			audio.removeEventListener("ended", onEnded);
			audio.pause();
			if (sourceRef.current) {
				try {
					sourceRef.current.disconnect();
				} catch {}
				sourceRef.current = null;
			}
		};
	}, [src, autoPlay, ensureGraph]);

	useEffect(
		() => () => {
			if (ctxRef.current) {
				ctxRef.current.close().catch(() => {});
				ctxRef.current = null;
			}
		},
		[],
	);

	const play = useCallback(() => {
		const audio = audioRef.current;
		if (!audio) return;
		ensureGraph();
		void ctxRef.current?.resume().catch(() => {});
		void audio.play().catch(() => {});
	}, [ensureGraph]);

	const pause = useCallback(() => {
		audioRef.current?.pause();
	}, []);

	const toggle = useCallback(() => {
		const audio = audioRef.current;
		if (!audio) return;
		if (audio.paused) play();
		else audio.pause();
	}, [play]);

	const seek = useCallback((seconds: number) => {
		const audio = audioRef.current;
		if (!audio) return;
		const target = Math.max(0, seconds);
		const max = Number.isFinite(audio.duration) ? audio.duration : target;
		audio.currentTime = Math.min(max, target);
		setCurrentTime(audio.currentTime);
	}, []);

	return {
		isPlaying,
		currentTime,
		duration,
		analyser,
		play,
		pause,
		toggle,
		seek,
	};
}
