import {
	BARGE_IN_CONFIRM_MS,
	BARGE_IN_COOLDOWN_MS,
	BARGE_IN_THRESHOLD,
	LISTEN_SAMPLE_RATE,
	type Utterance,
	createBargeInDetector,
	createUtteranceSegmenter,
	resample,
} from "./call-listening";
import {
	type CallTurnDetector,
	getCallTurnDetector,
} from "./call-turn-detector";

export { warmCallListening } from "./call-turn-detector";

export interface CallListener {
	/** While paused, captured audio is discarded and a half-heard utterance is dropped. */
	setPaused(paused: boolean): void;
	/**
	 * While guarded (a bot is speaking), nothing is transcribed unless a person clearly talks over it:
	 * that starts a barge-in, which `endBargeIn` then confirms or drops.
	 */
	setGuarded(guarded: boolean): void;
	/** Confirmed barge-ins carry on as a normal utterance; rejected ones are dropped and not retried for a while. */
	endBargeIn(confirmed: boolean): void;
	stop(): void;
}

const BLOCK_SAMPLES = 2048;
export const VAD_WAIT_MS = 150;
export const VAD_MAX_BACKLOG = 4;

let listenerSequence = 0;

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T | null> {
	return new Promise((resolve) => {
		const timer = setTimeout(() => resolve(null), ms);
		promise.then(
			(value) => {
				clearTimeout(timer);
				resolve(value);
			},
			() => {
				clearTimeout(timer);
				resolve(null);
			},
		);
	});
}

function acquirePageDetector(): {
	detector: CallTurnDetector;
	release: () => void;
} {
	const detector = getCallTurnDetector();
	return { detector, release: detector.acquire() };
}

const WORKLET_SOURCE = `
class Tap extends AudioWorkletProcessor {
	constructor() { super(); this.buffer = new Float32Array(${BLOCK_SAMPLES}); this.at = 0; }
	process(inputs) {
		const channel = inputs[0] && inputs[0][0];
		if (!channel) return true;
		for (let i = 0; i < channel.length; i++) {
			this.buffer[this.at++] = channel[i];
			if (this.at === this.buffer.length) {
				this.port.postMessage(this.buffer.slice());
				this.at = 0;
			}
		}
		return true;
	}
}
registerProcessor("call-listener-tap", Tap);
`;

const workletModules = new WeakMap<AudioContext, Promise<void>>();

async function loadTap(context: AudioContext): Promise<void> {
	let pending = workletModules.get(context);
	if (!pending) {
		const url = URL.createObjectURL(
			new Blob([WORKLET_SOURCE], { type: "text/javascript" }),
		);
		pending = context.audioWorklet
			.addModule(url)
			.finally(() => URL.revokeObjectURL(url));
		workletModules.set(context, pending);
	}
	try {
		await pending;
	} catch (cause) {
		if (workletModules.get(context) === pending) workletModules.delete(context);
		throw cause;
	}
}

async function attachTap(
	context: AudioContext,
	source: MediaStreamAudioSourceNode,
	onBlock: (block: Float32Array) => void,
): Promise<() => void> {
	const sink = context.createGain();
	sink.gain.value = 0;
	sink.connect(context.destination);
	if (context.audioWorklet) {
		try {
			await loadTap(context);
			const node = new AudioWorkletNode(context, "call-listener-tap");
			node.port.onmessage = (event: MessageEvent<Float32Array>) =>
				onBlock(event.data);
			source.connect(node);
			node.connect(sink);
			return () => {
				node.port.onmessage = null;
				node.disconnect();
				sink.disconnect();
			};
		} catch (cause) {
			console.warn("AudioWorklet unavailable, using a script processor", cause);
		}
	}
	try {
		const node = context.createScriptProcessor(BLOCK_SAMPLES, 1, 1);
		node.onaudioprocess = (event) =>
			onBlock(new Float32Array(event.inputBuffer.getChannelData(0)));
		source.connect(node);
		node.connect(sink);
		return () => {
			node.onaudioprocess = null;
			node.disconnect();
			sink.disconnect();
		};
	} catch (cause) {
		sink.disconnect();
		throw cause;
	}
}

/** Listens to a microphone stream and reports each spoken utterance as 16 kHz mono PCM. */
export async function startCallListener(
	stream: MediaStream,
	onUtterance: (utterance: Utterance) => void,
	options: {
		context?: AudioContext;
		paused?: boolean;
		onPartial?: (utterance: Utterance) => void;
		onReset?: () => void;
		guarded?: boolean;
		onBargeInStart?: () => void;
		/** Audio captured since the barge-in began, once enough is available to be worth transcribing. */
		onBargeInProbe?: (utterance: Utterance) => void;
		/** Voice-activity and turn models; null keeps the energy rule, undefined uses the page's shared models. */
		detector?: CallTurnDetector | null;
	} = {},
): Promise<CallListener> {
	// Reuse the context unlocked by the call gesture, including on mobile browsers.
	const context = options.context ?? new AudioContext();
	const ownsContext = options.context === undefined;
	const page = options.detector === undefined ? acquirePageDetector() : null;
	const turns = page?.detector ?? options.detector ?? null;
	const speechStream = `call-listener-${++listenerSequence}`;
	let source: MediaStreamAudioSourceNode | undefined;
	let detach: (() => void) | undefined;
	let processedSamples = 0;
	let clockOrigin: number | null = null;
	const captured = (utterance: Utterance): Utterance => {
		const origin = clockOrigin ?? Date.now();
		return {
			...utterance,
			capturedAt: origin + utterance.offsetMs,
			...(utterance.speechEndMs === undefined
				? {}
				: { endOfSpeechAt: origin + utterance.speechEndMs }),
		};
	};
	let paused = options.paused ?? false;
	let stopped = false;
	let guarded = options.guarded ?? false;
	let bargeIn: {
		parts: Float32Array[];
		length: number;
		probed: boolean;
		startedAt: number;
		held: Utterance[];
	} | null = null;
	let cooldownUntil = 0;
	const detector = createBargeInDetector({
		sampleRate: LISTEN_SAMPLE_RATE,
		threshold: BARGE_IN_THRESHOLD,
	});
	const probabilities = new WeakMap<Float32Array, number>();
	const emit = (utterance: Utterance) => {
		if (guarded && !bargeIn) return;
		if (bargeIn) bargeIn.held.push(utterance);
		else onUtterance(captured(utterance));
	};
	const segmenter = createUtteranceSegmenter({
		sampleRate: LISTEN_SAMPLE_RATE,
		onPartial: options.onPartial
			? (utterance) => {
					if (!guarded && !bargeIn) options.onPartial?.(captured(utterance));
				}
			: undefined,
		onTurnCheck: (id, samples) => {
			if (!turns?.turnReady) {
				turns?.wake?.();
				return false;
			}
			void turns.turnComplete(samples).then((probability) => {
				if (stopped) return;
				for (const utterance of segmenter.answerTurn(id, probability)) {
					emit(utterance);
				}
			});
			return true;
		},
	});
	const feed = (samples: Float32Array) => {
		for (const utterance of segmenter.push(
			samples,
			probabilities.get(samples),
		)) {
			emit(utterance);
		}
		processedSamples += samples.length;
	};
	/** When the audio clock started, given that `length` more samples had arrived by `arrivedAt`. */
	const originFor = (arrivedAt: number, length: number) =>
		arrivedAt - ((processedSamples + length) / LISTEN_SAMPLE_RATE) * 1000;
	const startBargeIn = (chunks: Float32Array[], arrivedAt: number) => {
		segmenter.reset();
		const length = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
		clockOrigin = originFor(arrivedAt, length);
		bargeIn = {
			parts: [],
			length: 0,
			probed: false,
			startedAt: arrivedAt,
			held: [],
		};
		options.onBargeInStart?.();
		for (const chunk of chunks) trackBarge(chunk);
	};
	const trackBarge = (chunk: Float32Array) => {
		if (!bargeIn) return;
		bargeIn.parts.push(chunk);
		bargeIn.length += chunk.length;
		feed(chunk);
		if (
			bargeIn &&
			!bargeIn.probed &&
			bargeIn.length >= (BARGE_IN_CONFIRM_MS * LISTEN_SAMPLE_RATE) / 1000
		) {
			bargeIn.probed = true;
			const samples = new Float32Array(bargeIn.length);
			let at = 0;
			for (const part of bargeIn.parts) {
				samples.set(part, at);
				at += part.length;
			}
			options.onBargeInProbe?.(
				captured({
					samples,
					offsetMs: Math.round(
						(processedSamples - bargeIn.length) / (LISTEN_SAMPLE_RATE / 1000),
					),
					durationMs: Math.round(bargeIn.length / (LISTEN_SAMPLE_RATE / 1000)),
				}),
			);
		}
	};

	const watchForBargeIn = (
		samples: Float32Array,
		arrivedAt: number,
		probability?: number,
	) => {
		if (arrivedAt < cooldownUntil) return;
		const chunks = detector.push(samples, probability);
		if (chunks) startBargeIn(chunks, arrivedAt);
	};

	const listenTo = (samples: Float32Array, arrivedAt: number) => {
		// The segmenter skips paused audio, so anchor its clock again after a pause.
		clockOrigin ??= originFor(arrivedAt, samples.length);
		if (bargeIn) trackBarge(samples);
		else feed(samples);
	};

	const handleBlock = (
		samples: Float32Array,
		arrivedAt: number,
		probability?: number,
	) => {
		if (stopped || paused) return;
		if (probability !== undefined) probabilities.set(samples, probability);
		if (guarded && !bargeIn) watchForBargeIn(samples, arrivedAt, probability);
		else listenTo(samples, arrivedAt);
	};

	const backlog: {
		samples: Float32Array;
		arrivedAt: number;
		probability: Promise<number | null> | null;
	}[] = [];
	let draining = false;
	const nextBlock = () => (stopped ? undefined : backlog.shift());
	const drain = async () => {
		if (draining) return;
		draining = true;
		try {
			for (let item = nextBlock(); item; item = nextBlock()) {
				const probability =
					item.probability && backlog.length < VAD_MAX_BACKLOG
						? await withTimeout(item.probability, VAD_WAIT_MS)
						: null;
				handleBlock(item.samples, item.arrivedAt, probability ?? undefined);
			}
		} finally {
			draining = false;
		}
	};
	// Blocks are handled in arrival order; a slow or dead model costs that block its model answer, never its place.
	const receive = (samples: Float32Array) => {
		const arrivedAt = Date.now();
		if (turns && !turns.ready) turns.wake?.();
		const probability = turns?.ready
			? turns.speech(samples, speechStream)
			: null;
		if (!draining && !probability) {
			handleBlock(samples, arrivedAt);
			return;
		}
		backlog.push({ samples, arrivedAt, probability });
		void drain();
	};

	const release = () => {
		page?.release();
		detach?.();
		try {
			source?.disconnect();
		} catch {}
		if (ownsContext) void context.close().catch(() => {});
	};
	try {
		if (context.state !== "running") await context.resume();
		if (context.state !== "running")
			throw new Error("Call microphone capture is unavailable");
		source = context.createMediaStreamSource(stream);
		detach = await attachTap(context, source, (block) => {
			if (stopped || paused) return;
			receive(resample(block, context.sampleRate, LISTEN_SAMPLE_RATE));
		});
	} catch (cause) {
		release();
		throw cause;
	}

	return {
		setPaused(next) {
			if (stopped || next === paused) return;
			paused = next;
			if (next) {
				segmenter.reset();
				detector.reset();
				turns?.resetSpeech(speechStream);
				bargeIn = null;
				clockOrigin = null;
				options.onReset?.();
			}
		},
		setGuarded(next) {
			if (stopped || next === guarded) return;
			guarded = next;
			detector.reset();
			if (next) {
				segmenter.reset();
				bargeIn = null;
				clockOrigin = null;
				options.onReset?.();
			}
		},
		endBargeIn(confirmed) {
			const current = bargeIn;
			if (stopped || !current) return;
			bargeIn = null;
			if (!confirmed) {
				segmenter.reset();
				clockOrigin = null;
				detector.reset();
				cooldownUntil = Date.now() + BARGE_IN_COOLDOWN_MS;
				return;
			}
			guarded = false;
			for (const utterance of current.held) onUtterance(captured(utterance));
		},
		stop() {
			if (stopped) return;
			for (const item of backlog.splice(0)) {
				handleBlock(item.samples, item.arrivedAt);
			}
			stopped = true;
			turns?.resetSpeech(speechStream);
			const last = segmenter.flush();
			if (last && !guarded) onUtterance(captured(last));
			options.onReset?.();
			release();
		},
	};
}
