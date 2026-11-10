import type { SpeechTimers } from "./bot-call-speech";

export const CUE_PEAK_GAIN = 0.05;
export const CUE_INTERVAL_MS = 2400;
const TONES = [
	{ frequency: 660, offset: 0 },
	{ frequency: 880, offset: 0.14 },
] as const;
const ATTACK_SECONDS = 0.015;
const TONE_SECONDS = 0.12;

const defaultTimers: SpeechTimers = {
	set: (callback, ms) => setTimeout(callback, ms),
	clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

/** Soft cues for the local speakers only; they never enter the outgoing call stream. */
export function createCallCues(
	context: AudioContext,
	destination: AudioNode,
	timers: SpeechTimers = defaultTimers,
) {
	let active = false;
	let timer: unknown = null;
	let disposed = false;

	const tone = (frequency: number, at: number) => {
		const oscillator = context.createOscillator();
		const envelope = context.createGain();
		oscillator.type = "sine";
		oscillator.frequency.value = frequency;
		envelope.gain.setValueAtTime(0, at);
		envelope.gain.linearRampToValueAtTime(CUE_PEAK_GAIN, at + ATTACK_SECONDS);
		envelope.gain.linearRampToValueAtTime(0, at + TONE_SECONDS);
		oscillator.connect(envelope);
		envelope.connect(destination);
		oscillator.onended = () => {
			oscillator.disconnect();
			envelope.disconnect();
		};
		oscillator.start(at);
		oscillator.stop(at + TONE_SECONDS + 0.02);
	};

	const tick = () => {
		if (!active || disposed || context.state !== "running") return;
		try {
			for (const { frequency, offset } of TONES)
				tone(frequency, context.currentTime + offset);
		} catch {}
	};

	const loop = () => {
		tick();
		timer = timers.set(loop, CUE_INTERVAL_MS);
	};

	const stop = () => {
		active = false;
		if (timer !== null) timers.clear(timer);
		timer = null;
	};

	return {
		thinking: {
			start() {
				if (active || disposed) return;
				active = true;
				loop();
			},
			stop,
		},
		dispose() {
			disposed = true;
			stop();
		},
	};
}

export type CallCues = ReturnType<typeof createCallCues>;
