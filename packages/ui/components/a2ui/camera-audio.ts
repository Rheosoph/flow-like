import { CameraError } from "./camera-overlays";

export const CAMERA_AUDIO_SAMPLE_RATE = 16_000;

export function validateAudioBufferSeconds(value: number) {
	if (!Number.isInteger(value) || value < 1 || value > 300)
		throw new CameraError(
			"invalid_arguments",
			"Audio history must be 1 to 300 whole seconds.",
		);
	return value;
}

export function validateAudioDuration(value: number) {
	if (!Number.isFinite(value) || value < 100 || value > 300_000)
		throw new CameraError(
			"invalid_arguments",
			"Audio capture duration must be 100 to 300000 milliseconds.",
		);
	return value;
}

/** This self-contained function also runs in the audio worklet. Times use its audio clock. */
export function createCameraPcmRing(inputRate: number, seconds: number) {
	const rate = 16_000;
	const capacity = Math.round(seconds * rate);
	const samples = new Float32Array(capacity);
	const ratio = inputRate / rate;
	let total = 0;
	let weight = 0;
	let sum = 0;
	let origin: number | undefined;
	return {
		append(channels: Float32Array[], time: number) {
			if (!channels[0]?.length) return;
			origin ??= time;
			for (let i = 0; i < channels[0].length; i++) {
				let sample = 0;
				for (const channel of channels) sample += channel[i] ?? 0;
				sample = Math.max(-1, Math.min(1, sample / channels.length));
				let remaining = 1;
				while (remaining > 1e-9) {
					const take = Math.min(remaining, ratio - weight);
					sum += sample * take;
					weight += take;
					remaining -= take;
					if (weight + 1e-9 >= ratio) {
						samples[total++ % capacity] = sum / ratio;
						weight = 0;
						sum = 0;
					}
				}
			}
		},
		snapshot(durationMs: number, cutoff: number) {
			const end = Math.max(
				0,
				Math.min(
					total,
					Math.floor((cutoff - (origin ?? cutoff)) * rate + 1e-6),
				),
			);
			const start = Math.max(
				0,
				total - capacity,
				end - Math.floor((durationMs * rate) / 1000),
			);
			const pcm = new Float32Array(Math.max(0, end - start));
			for (let i = 0; i < pcm.length; i++)
				pcm[i] = samples[(start + i) % capacity];
			return { pcm, endTime: (origin ?? cutoff) + end / rate };
		},
		availableMs() {
			return (Math.min(total, capacity) / rate) * 1000;
		},
		clear() {
			samples.fill(0);
			total = 0;
			weight = 0;
			sum = 0;
			origin = undefined;
		},
	};
}

export function cameraAudioWorkletSource() {
	return `
const createRing = ${createCameraPcmRing.toString()};
class CameraAudioProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.ring = createRing(sampleRate, options.processorOptions.bufferSeconds);
    this.active = true;
    this.epoch = 0;
    this.lastReport = 0;
    this.port.onmessage = ({data}) => {
      if (data.type === 'pause' || data.type === 'resume') {
        this.ring.clear();
        this.active = data.type === 'resume';
        this.epoch = data.epoch;
      } else if (data.type === 'snapshot' && this.active) {
        const snapshot = this.ring.snapshot(data.durationMs, data.cutoff);
        this.port.postMessage({id: data.id, epoch: this.epoch, ...snapshot}, [snapshot.pcm.buffer]);
      }
    };
  }
  process(inputs) {
    if (this.active) this.ring.append(inputs[0] || [], currentFrame / sampleRate);
    if (currentFrame - this.lastReport >= sampleRate / 4) {
      this.lastReport = currentFrame;
      this.port.postMessage({availableMs: this.ring.availableMs(), epoch: this.epoch});
    }
    return true;
  }
}
registerProcessor('flow-like-camera-audio', CameraAudioProcessor);`;
}

export interface CameraAudioSnapshot {
	pcm: Float32Array;
	startedAt: number;
	endedAt: number;
}

export interface CameraAudioRecorder {
	snapshot(
		durationMs: number,
		capturedAt: number,
	): Promise<CameraAudioSnapshot>;
	pause(): void;
	resume(): void;
	dispose(): void;
}

interface RecorderOptions {
	signal: AbortSignal;
	onAvailable: (milliseconds: number) => void;
	onError: (error: CameraError) => void;
}

export async function createCameraAudioRecorder(
	stream: MediaStream | Promise<MediaStream>,
	bufferSeconds: number,
	options: RecorderOptions,
): Promise<CameraAudioRecorder> {
	validateAudioBufferSeconds(bufferSeconds);
	if (options.signal.aborted)
		throw new CameraError(
			"screen_closed",
			"The camera screen is no longer active.",
		);
	if (
		typeof AudioContext === "undefined" ||
		typeof AudioWorkletNode === "undefined"
	)
		throw new CameraError(
			"unsupported",
			"Rolling audio needs Web Audio with AudioWorklet support.",
		);
	const context = new AudioContext();
	let node: AudioWorkletNode | undefined;
	let source: MediaStreamAudioSourceNode | undefined;
	let disposed = false;
	let active = true;
	let sequence = 0;
	let epoch = 0;
	const pending = new Map<
		number,
		{
			resolve: (snapshot: CameraAudioSnapshot) => void;
			reject: (error: unknown) => void;
			cutoff: number;
			capturedAt: number;
			timer: ReturnType<typeof setTimeout>;
		}
	>();
	const cancelPending = () => {
		for (const item of pending.values()) {
			clearTimeout(item.timer);
			item.reject(
				new CameraError(
					"audio_not_live",
					"Microphone recording is no longer active.",
				),
			);
		}
		pending.clear();
	};
	const dispose = () => {
		if (disposed) return;
		disposed = true;
		active = false;
		cancelPending();
		source?.disconnect();
		if (node) {
			node.port.postMessage({ type: "pause", epoch: ++epoch });
			node.port.onmessage = null;
			node.port.close();
			node.disconnect();
		}
		void context.close().catch(() => {});
		options.signal.removeEventListener("abort", dispose);
		context.removeEventListener("statechange", interrupted);
	};
	const interrupted = () => {
		if (!disposed && node && context.state !== "running") {
			dispose();
			options.onError(
				new CameraError(
					"audio_unavailable",
					"Microphone audio was interrupted. Start the camera again.",
				),
			);
		}
	};
	options.signal.addEventListener("abort", dispose, { once: true });
	context.addEventListener("statechange", interrupted);
	let moduleUrl: string | undefined;
	try {
		moduleUrl = URL.createObjectURL(
			new Blob([cameraAudioWorkletSource()], { type: "text/javascript" }),
		);
		// Resume during the explicit Start gesture, before waiting for worklet compilation.
		const resumed = context.resume();
		const [, , media] = await Promise.all([
			resumed,
			context.audioWorklet.addModule(moduleUrl),
			stream,
		]);
		if (disposed || options.signal.aborted)
			throw new CameraError(
				"screen_closed",
				"The camera screen closed while starting its microphone.",
			);
		if (context.state !== "running")
			throw new CameraError(
				"audio_unavailable",
				"Microphone audio was interrupted before recording started.",
			);
		node = new AudioWorkletNode(context, "flow-like-camera-audio", {
			numberOfInputs: 1,
			numberOfOutputs: 1,
			outputChannelCount: [1],
			channelCount: 1,
			channelCountMode: "explicit",
			processorOptions: { bufferSeconds },
		});
		node.onprocessorerror = () => {
			dispose();
			options.onError(
				new CameraError(
					"audio_unavailable",
					"Microphone processing stopped. Start the camera again.",
				),
			);
		};
		node.port.onmessage = ({ data }) => {
			if (disposed || !active || data.epoch !== epoch) return;
			if (typeof data.availableMs === "number") {
				options.onAvailable(data.availableMs);
				return;
			}
			const item = pending.get(data.id);
			if (!item) return;
			pending.delete(data.id);
			clearTimeout(item.timer);
			if (!(data.pcm instanceof Float32Array) || !data.pcm.length) {
				item.reject(
					new CameraError(
						"audio_not_ready",
						"Wait for the first microphone samples.",
					),
				);
				return;
			}
			const endedAt = item.capturedAt + (data.endTime - item.cutoff) * 1000;
			item.resolve({
				pcm: data.pcm,
				endedAt,
				startedAt:
					endedAt - (data.pcm.length / CAMERA_AUDIO_SAMPLE_RATE) * 1000,
			});
		};
		source = context.createMediaStreamSource(media);
		source.connect(node);
		// The processor leaves its output silent; connecting it keeps capture rendering active.
		node.connect(context.destination);
		return {
			snapshot(durationMs, capturedAt) {
				validateAudioDuration(durationMs);
				if (disposed || !active || options.signal.aborted)
					return Promise.reject(
						new CameraError(
							"audio_not_live",
							"Microphone recording is no longer active.",
						),
					);
				const id = ++sequence;
				const cutoff = context.currentTime;
				return new Promise((resolve, reject) => {
					const timer = setTimeout(() => {
						pending.delete(id);
						reject(
							new CameraError(
								"audio_unavailable",
								"The microphone did not provide its audio history.",
							),
						);
					}, 3000);
					pending.set(id, { resolve, reject, cutoff, capturedAt, timer });
					node?.port.postMessage({ type: "snapshot", id, cutoff, durationMs });
				});
			},
			pause() {
				active = false;
				cancelPending();
				node?.port.postMessage({ type: "pause", epoch: ++epoch });
				options.onAvailable(0);
			},
			resume() {
				if (disposed)
					throw new CameraError(
						"audio_not_live",
						"Start the camera again to record microphone audio.",
					);
				node?.port.postMessage({ type: "resume", epoch: ++epoch });
				active = true;
				options.onAvailable(0);
			},
			dispose,
		};
	} catch (error) {
		dispose();
		throw error;
	} finally {
		if (moduleUrl) URL.revokeObjectURL(moduleUrl);
	}
}

/** Every clip is independently playable, including clips sliced from a wrapped ring. */
export function encodeCameraWav(pcm: Float32Array): ArrayBuffer {
	const buffer = new ArrayBuffer(44 + pcm.length * 2);
	const view = new DataView(buffer);
	const label = (offset: number, text: string) => {
		for (let i = 0; i < text.length; i++)
			view.setUint8(offset + i, text.charCodeAt(i));
	};
	label(0, "RIFF");
	view.setUint32(4, buffer.byteLength - 8, true);
	label(8, "WAVE");
	label(12, "fmt ");
	view.setUint32(16, 16, true);
	view.setUint16(20, 1, true);
	view.setUint16(22, 1, true);
	view.setUint32(24, CAMERA_AUDIO_SAMPLE_RATE, true);
	view.setUint32(28, CAMERA_AUDIO_SAMPLE_RATE * 2, true);
	view.setUint16(32, 2, true);
	view.setUint16(34, 16, true);
	label(36, "data");
	view.setUint32(40, pcm.length * 2, true);
	for (let i = 0; i < pcm.length; i++) {
		const sample = Number.isFinite(pcm[i])
			? Math.max(-1, Math.min(1, pcm[i]))
			: 0;
		view.setInt16(
			44 + i * 2,
			Math.round(sample * (sample < 0 ? 32768 : 32767)),
			true,
		);
	}
	return buffer;
}
