import { atom } from "nanostores";
import type {
	FlwEnvelope,
	FlwMessageType,
	FlwPayloadMap,
	InitCapabilities,
} from "./protocol";
export interface WidgetPublicMediaGrant {
	id: string;
	url: string;
}
export interface WidgetMediaState {
	id?: string;
	state: "idle" | "loading" | "playing" | "paused" | "error";
	error?: string;
}
export interface MediaPlayPayload {
	requestId: string;
	mediaId: string;
}
export interface MediaResultPayload {
	requestId: string;
	ok: boolean;
}
export function createWidgetMediaClient(
	post: <T extends FlwMessageType>(type: T, payload: FlwPayloadMap[T]) => void,
	capabilities: () => InitCapabilities,
) {
	const $media = atom<WidgetMediaState>({ state: "idle" });
	let sequence = 0;
	let disposed = false;
	const pending = new Map<
		string,
		{
			resolve: () => void;
			reject: (error: Error) => void;
			timer: ReturnType<typeof setTimeout>;
		}
	>();
	const settle = (id: string, ok: boolean) => {
		const row = pending.get(id);
		if (!row) return;
		clearTimeout(row.timer);
		pending.delete(id);
		if (ok) row.resolve();
		else
			row.reject(
				new Error("Media playback unavailable; use the host playback control"),
			);
	};
	return {
		$media,
		playMedia(id: string): Promise<void> {
			if (
				disposed ||
				!capabilities().media ||
				!capabilities().mediaIds?.includes(id)
			)
				return Promise.reject(new Error("Media is not granted to this widget"));
			if (pending.size >= 4)
				return Promise.reject(
					new Error("Media playback request already pending"),
				);
			const requestId = `media-${++sequence}`;
			return new Promise((resolve, reject) => {
				pending.set(requestId, {
					resolve,
					reject,
					timer: setTimeout(() => settle(requestId, false), 15000),
				});
				post("media:play", { requestId, mediaId: id });
			});
		},
		pauseMedia() {
			if (!disposed) post("media:pause", {});
		},
		stopMedia() {
			if (!disposed) post("media:stop", {});
		},
		handle(envelope: FlwEnvelope) {
			if (envelope.type === "media:result") {
				const result = envelope.payload as MediaResultPayload;
				if (result && typeof result.requestId === "string")
					settle(result.requestId, result.ok === true);
			} else if (envelope.type === "media:state") {
				const state = envelope.payload as WidgetMediaState;
				if (
					state &&
					["idle", "loading", "playing", "paused", "error"].includes(
						state.state,
					)
				)
					$media.set(state);
			}
		},
		reset() {
			for (const id of pending.keys()) settle(id, false);
			$media.set({ state: "idle" });
		},
		dispose() {
			disposed = true;
			post("media:stop", {});
			for (const id of pending.keys()) settle(id, false);
			$media.set({ state: "idle" });
		},
	};
}
