export type CallVideoKind = "camera" | "screen";
export type CallMediaFailure = "denied" | "unavailable" | "failed";
export interface CallMediaError {
	source: CallVideoKind;
	reason: CallMediaFailure;
}

export interface VideoCaptureState {
	camera: MediaStream | null;
	screen: MediaStream | null;
	pending: CallVideoKind | null;
	error: CallMediaError | null;
}

export function mediaFailure(cause: unknown): CallMediaFailure {
	const name =
		typeof cause === "object" && cause !== null && "name" in cause
			? cause.name
			: "";
	if (name === "NotAllowedError" || name === "SecurityError") return "denied";
	if (
		[
			"NotFoundError",
			"NotReadableError",
			"OverconstrainedError",
			"TypeError",
			"NotSupportedError",
		].includes(String(name))
	)
		return "unavailable";
	return "failed";
}

export function captureCallVideo(kind: CallVideoKind): Promise<MediaStream> {
	const devices =
		typeof navigator !== "undefined" ? navigator.mediaDevices : undefined;
	if (kind === "screen") {
		if (!devices?.getDisplayMedia)
			return Promise.reject(
				new DOMException("Screen sharing is unavailable", "NotSupportedError"),
			);
		// Called directly from the button handler to retain browser user activation.
		return devices.getDisplayMedia({
			video: { frameRate: { ideal: 15, max: 30 } },
			audio: false,
		});
	}
	if (!devices?.getUserMedia)
		return Promise.reject(
			new DOMException("Camera capture is unavailable", "NotSupportedError"),
		);
	return devices.getUserMedia({
		video: {
			width: { ideal: 640 },
			height: { ideal: 360 },
			frameRate: { ideal: 24, max: 30 },
		},
		audio: false,
	});
}

/** Owns camera/screen tracks, including permission prompts that resolve after leaving. */
export function createVideoCapture(
	onChange: (state: VideoCaptureState) => void,
	capture = captureCallVideo,
) {
	const state: VideoCaptureState = {
		camera: null,
		screen: null,
		pending: null,
		error: null,
	};
	const cleanups = new Map<CallVideoKind, () => void>();
	let disposed = false;
	const emit = () => {
		if (!disposed) onChange({ ...state });
	};
	const stop = (kind: CallVideoKind) => {
		cleanups.get(kind)?.();
		cleanups.delete(kind);
		for (const track of state[kind]?.getTracks() ?? []) track.stop();
		state[kind] = null;
	};
	return {
		get state(): Readonly<VideoCaptureState> {
			return state;
		},
		async toggle(kind: CallVideoKind) {
			if (disposed || state.pending) return;
			state.error = null;
			if (state[kind]) {
				stop(kind);
				emit();
				return;
			}
			state.pending = kind;
			emit();
			try {
				const stream = await capture(kind);
				if (disposed) {
					for (const track of stream.getTracks()) track.stop();
					return;
				}
				if (
					!stream.getVideoTracks().some((track) => track.readyState !== "ended")
				) {
					for (const track of stream.getTracks()) track.stop();
					throw new DOMException(
						"No video track was captured",
						"NotFoundError",
					);
				}
				state[kind] = stream;
				const ended = () => {
					if (!disposed && state[kind] === stream) {
						stop(kind);
						emit();
					}
				};
				for (const track of stream.getVideoTracks())
					track.addEventListener("ended", ended);
				cleanups.set(kind, () => {
					for (const track of stream.getVideoTracks())
						track.removeEventListener("ended", ended);
				});
			} catch (cause) {
				if (!disposed)
					state.error = { source: kind, reason: mediaFailure(cause) };
			} finally {
				state.pending = null;
				emit();
			}
		},
		dispose() {
			if (disposed) return;
			disposed = true;
			stop("camera");
			stop("screen");
		},
	};
}
