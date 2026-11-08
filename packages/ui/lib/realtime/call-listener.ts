import {
	LISTEN_SAMPLE_RATE,
	type Utterance,
	createUtteranceSegmenter,
	resample,
} from "./call-listening";

export interface CallListener {
	/** While paused, captured audio is discarded and a half-heard utterance is dropped. */
	setPaused(paused: boolean): void;
	stop(): void;
}

const BLOCK_SAMPLES = 2048;

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
	options: { context?: AudioContext; paused?: boolean } = {},
): Promise<CallListener> {
	// Reuse the context unlocked by the call gesture, including on mobile browsers.
	const context = options.context ?? new AudioContext();
	const ownsContext = options.context === undefined;
	let source: MediaStreamAudioSourceNode | undefined;
	let detach: (() => void) | undefined;
	const segmenter = createUtteranceSegmenter({
		sampleRate: LISTEN_SAMPLE_RATE,
	});
	let paused = options.paused ?? false;
	let stopped = false;

	const release = () => {
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
			for (const utterance of segmenter.push(
				resample(block, context.sampleRate, LISTEN_SAMPLE_RATE),
			)) {
				onUtterance(utterance);
			}
		});
	} catch (cause) {
		release();
		throw cause;
	}

	return {
		setPaused(next) {
			if (next === paused) return;
			paused = next;
			if (next) segmenter.reset();
		},
		stop() {
			if (stopped) return;
			stopped = true;
			const last = segmenter.flush();
			if (last) onUtterance(last);
			release();
		},
	};
}
