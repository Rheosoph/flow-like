import {
	LISTEN_SAMPLE_RATE,
	type Utterance,
	createUtteranceSegmenter,
	resample,
} from "./call-listening";

export interface CallListener {
	/** While paused (muted) captured audio is discarded and a half-heard utterance is dropped. */
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

async function attachTap(
	context: AudioContext,
	source: MediaStreamAudioSourceNode,
	onBlock: (block: Float32Array) => void,
): Promise<() => void> {
	const sink = context.createGain();
	sink.gain.value = 0;
	sink.connect(context.destination);
	if (context.audioWorklet) {
		const url = URL.createObjectURL(
			new Blob([WORKLET_SOURCE], { type: "text/javascript" }),
		);
		try {
			await context.audioWorklet.addModule(url);
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
		} finally {
			URL.revokeObjectURL(url);
		}
	}
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
}

/** Listens to a microphone stream and reports each spoken utterance as 16 kHz mono PCM. */
export async function startCallListener(
	stream: MediaStream,
	onUtterance: (utterance: Utterance) => void,
): Promise<CallListener> {
	const context = new AudioContext();
	await context.resume().catch(() => {});
	const source = context.createMediaStreamSource(stream);
	const segmenter = createUtteranceSegmenter({
		sampleRate: LISTEN_SAMPLE_RATE,
	});
	let paused = false;
	let stopped = false;

	const detach = await attachTap(context, source, (block) => {
		if (stopped) return;
		if (paused) return;
		for (const utterance of segmenter.push(
			resample(block, context.sampleRate, LISTEN_SAMPLE_RATE),
		)) {
			onUtterance(utterance);
		}
	});

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
			detach();
			try {
				source.disconnect();
			} catch {}
			void context.close().catch(() => {});
		},
	};
}
