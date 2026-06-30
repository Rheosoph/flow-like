"use client";

import { useEffect, useRef } from "react";

export interface UseSpeakerActivityOptions {
	analyser: AnalyserNode | null;
	/** Whether detection should run. */
	active: boolean;
	/** RMS amplitude below which the signal counts as silence. */
	silenceThreshold?: number;
	/** Sustained silence (ms) after speech before `onSilence` fires. */
	silenceDuration?: number;
	/** Grace period (ms) before detection starts, to skip leading silence. */
	startDelay?: number;
	/** Fired once when the speaker has finished (silence after speech). */
	onSilence?: () => void;
	/** Fired when speech is first detected. */
	onSpeechStart?: () => void;
}

/**
 * Frontend speaker / pause detection over an AnalyserNode. Powers auto-stop and
 * the automatic invoke mode (pause = finished talking) plus interrupt detection
 * (speech started while something else is happening). Extracted and generalized
 * from the RMS silence loops in VoiceMode and VoiceInput.
 */
export function useSpeakerActivity({
	analyser,
	active,
	silenceThreshold = 0.01,
	silenceDuration = 2000,
	startDelay = 1200,
	onSilence,
	onSpeechStart,
}: UseSpeakerActivityOptions): void {
	const silenceCb = useRef(onSilence);
	const speechCb = useRef(onSpeechStart);
	silenceCb.current = onSilence;
	speechCb.current = onSpeechStart;

	useEffect(() => {
		if (!analyser || !active) return;
		let cancelled = false;
		let silentSince: number | null = null;
		let speaking = false;
		let fired = false;
		let timeout: ReturnType<typeof setTimeout> | null = null;
		const data = new Float32Array(analyser.fftSize);
		const startedAt = Date.now();

		const tick = () => {
			if (cancelled) return;
			analyser.getFloatTimeDomainData(data);
			let sum = 0;
			for (let i = 0; i < data.length; i++) sum += data[i] * data[i];
			const rms = Math.sqrt(sum / data.length);

			if (Date.now() - startedAt >= startDelay) {
				if (rms < silenceThreshold) {
					if (!silentSince) silentSince = Date.now();
					if (
						speaking &&
						!fired &&
						Date.now() - silentSince > silenceDuration
					) {
						fired = true;
						silenceCb.current?.();
						return;
					}
				} else {
					silentSince = null;
					if (!speaking) {
						speaking = true;
						speechCb.current?.();
					}
				}
			}
			timeout = setTimeout(tick, 100);
		};
		tick();

		return () => {
			cancelled = true;
			if (timeout) clearTimeout(timeout);
		};
	}, [analyser, active, silenceThreshold, silenceDuration, startDelay]);
}
