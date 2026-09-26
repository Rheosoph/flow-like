import type {
	FlwEnvelope,
	FlwMessageType,
	FlwPayloadMap,
	InitCapabilities,
} from "./protocol";

export interface WidgetAudioOptions {
	maxDurationMs?: number;
	signal?: AbortSignal;
}
export interface WidgetAudioResult {
	bytes: ArrayBuffer;
	mimeType: string;
	status: number;
}
export interface AudioRequestPayload {
	requestId: string;
	maxDurationMs: number;
}
export interface MicrophoneResultPayload {
	requestId: string;
	ok: boolean;
	bytes?: ArrayBuffer;
	mimeType?: string;
	status?: number;
	error?: string;
}
export interface RequestIdPayload {
	requestId: string;
}

export function createWidgetMicrophoneClient(
	post: <T extends FlwMessageType>(type: T, payload: FlwPayloadMap[T]) => void,
	capabilities: () => InitCapabilities,
) {
	let sequence = 0;
	let disposed = false;
	let audioId: string | undefined;
	const pending = new Map<
		string,
		{
			resolve: (value: WidgetAudioResult) => void;
			reject: (reason: Error) => void;
			cleanup: () => void;
		}
	>();
	function settle(id: string, value: Error | WidgetAudioResult) {
		const entry = pending.get(id);
		if (!entry) return;
		pending.delete(id);
		entry.cleanup();
		if (value instanceof Error) entry.reject(value);
		else entry.resolve(value);
	}
	return {
		captureAudio(options: WidgetAudioOptions = {}): Promise<WidgetAudioResult> {
			if (!capabilities().microphone)
				return Promise.reject(
					new Error("Microphone is not granted to this widget"),
				);
			if (audioId)
				return Promise.reject(
					new Error("Microphone capture is already active"),
				);
			const maxDurationMs = Math.max(
				1000,
				Math.min(60_000, options.maxDurationMs ?? 30_000),
			);
			if (!Number.isFinite(maxDurationMs))
				return Promise.reject(new Error("Invalid recording duration"));
			if (disposed) return Promise.reject(new Error("Widget is disposed"));
			const signal = options.signal;
			if (signal?.aborted)
				return Promise.reject(
					new DOMException("Request aborted", "AbortError"),
				);
			const requestId = `microphone-${++sequence}`;
			audioId = requestId;
			return new Promise((resolve, reject) => {
				const cancel = () => {
					post("microphone:cancel", { requestId });
					settle(requestId, new DOMException("Request aborted", "AbortError"));
				};
				const timer = setTimeout(() => {
					post("microphone:cancel", { requestId });
					settle(requestId, new Error("Widget request timed out"));
				}, 75_000);
				const cleanup = () => {
					clearTimeout(timer);
					signal?.removeEventListener("abort", cancel);
					if (audioId === requestId) audioId = undefined;
				};
				pending.set(requestId, { resolve, reject, cleanup });
				signal?.addEventListener("abort", cancel, { once: true });
				post("microphone:request", { requestId, maxDurationMs });
			});
		},
		stopAudioCapture() {
			if (audioId) post("microphone:stop", { requestId: audioId });
		},
		handle(envelope: FlwEnvelope) {
			if (envelope.type !== "microphone:result") return;
			const value = envelope.payload as MicrophoneResultPayload;
			if (!value || typeof value.requestId !== "string") return;
			if (
				value.ok !== true ||
				!(value.bytes instanceof ArrayBuffer) ||
				typeof value.mimeType !== "string" ||
				!Number.isInteger(value.status)
			) {
				settle(value.requestId, new Error("Microphone capture failed"));
				return;
			}
			settle(value.requestId, {
				bytes: value.bytes,
				mimeType: value.mimeType,
				status: value.status!,
			});
		},
		reset() {
			for (const id of [...pending.keys()])
				settle(id, new Error("Widget connection replaced"));
		},
		dispose() {
			disposed = true;
			for (const id of [...pending.keys()]) {
				post("microphone:cancel", { requestId: id });
				settle(id, new Error("Widget is disposed"));
			}
		},
	};
}
