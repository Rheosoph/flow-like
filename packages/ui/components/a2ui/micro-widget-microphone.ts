import type {
	AudioRequestPayload,
	FlwEnvelope,
	MicrophoneResultPayload,
} from "@flow-like/widget-sdk";

const MAX_AUDIO_BYTES = 8 * 1024 * 1024;
export function createMicroWidgetMicrophone(options: {
	enabled: () => boolean;
	result: (payload: MicrophoneResultPayload, transfer?: Transferable[]) => void;
	recording: (active: boolean) => void;
	beforeCapture?: () => void;
	getUserMedia?: () => Promise<MediaStream>;
	createRecorder?: (stream: MediaStream) => MediaRecorder;
}) {
	let disposed = false;
	let generation = 0;
	type Capture = {
		requestId: string;
		stream?: MediaStream;
		recorder?: MediaRecorder;
		timer?: ReturnType<typeof setTimeout>;
		cancelled: boolean;
	};
	let active: Capture | undefined;
	const fail = (requestId: string) =>
		options.result({
			requestId,
			ok: false,
			error: "Microphone capture unavailable",
		});
	function cancel(report = false) {
		generation++;
		const previous = active;
		active = undefined;
		if (!previous) return;
		previous.cancelled = true;
		clearTimeout(previous.timer);
		if (previous.recorder?.state === "recording") previous.recorder.stop();
		previous.stream?.getTracks().forEach((track) => track.stop());
		options.recording(false);
		if (report && !disposed) fail(previous.requestId);
	}
	async function start(payload: AudioRequestPayload) {
		if (
			disposed ||
			!options.enabled() ||
			active ||
			!Number.isFinite(payload.maxDurationMs)
		) {
			fail(payload.requestId);
			return;
		}
		const owner = ++generation;
		const capture = {
			requestId: payload.requestId,
			cancelled: false,
		} as Capture;
		active = capture;
		try {
			options.beforeCapture?.();
			const stream = await (options.getUserMedia?.() ??
				navigator.mediaDevices.getUserMedia({ audio: true }));
			if (disposed || owner !== generation || capture.cancelled) {
				stream.getTracks().forEach((track) => track.stop());
				return;
			}
			capture.stream = stream;
			const recorder =
				options.createRecorder?.(stream) ?? new MediaRecorder(stream);
			capture.recorder = recorder;
			const chunks: Blob[] = [];
			let bytes = 0;
			recorder.ondataavailable = (event) => {
				bytes += event.data.size;
				if (bytes > MAX_AUDIO_BYTES) {
					cancel(true);
					return;
				}
				if (!capture.cancelled) chunks.push(event.data);
			};
			recorder.onerror = () => cancel(true);
			recorder.onstop = () => {
				clearTimeout(capture.timer);
				stream.getTracks().forEach((track) => track.stop());
				if (active === capture) active = undefined;
				options.recording(false);
				if (disposed || capture.cancelled || owner !== generation) return;
				void new Blob(chunks, { type: recorder.mimeType })
					.arrayBuffer()
					.then((buffer) => {
						if (disposed || owner !== generation) return;
						options.result(
							{
								requestId: payload.requestId,
								ok: true,
								bytes: buffer,
								mimeType: recorder.mimeType || "audio/webm",
								status: 200,
							},
							[buffer],
						);
					})
					.catch(() => {
						if (!disposed && owner === generation) fail(payload.requestId);
					});
			};
			recorder.start(250);
			options.recording(true);
			capture.timer = setTimeout(
				() => {
					if (recorder.state === "recording") recorder.stop();
				},
				Math.max(1000, Math.min(60_000, payload.maxDurationMs)),
			);
		} catch {
			if (owner === generation) cancel(true);
		}
	}
	return {
		handle(envelope: FlwEnvelope) {
			const payload = envelope.payload as AudioRequestPayload;
			if (
				!payload ||
				typeof payload.requestId !== "string" ||
				!/^[a-zA-Z0-9_-]{1,128}$/.test(payload.requestId)
			)
				return;
			if (envelope.type === "microphone:request") void start(payload);
			else if (
				envelope.type === "microphone:cancel" &&
				active?.requestId === payload.requestId
			)
				cancel();
			else if (
				envelope.type === "microphone:stop" &&
				active?.requestId === payload.requestId
			) {
				if (active.recorder?.state === "recording") active.recorder.stop();
				// A hold can end while the permission prompt is still pending.
				// Discard that request so a late stream never starts recording.
				else if (!active.recorder) cancel(true);
			}
		},
		stop() {
			if (active?.recorder?.state === "recording") active.recorder.stop();
			else cancel(true);
		},
		revoke() {
			cancel(true);
		},
		dispose() {
			disposed = true;
			cancel();
		},
	};
}
