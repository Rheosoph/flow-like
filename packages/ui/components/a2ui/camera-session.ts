import type {
	ITemporaryFlowPath,
	ITemporaryUploadExecutionTarget,
} from "../../state/backend-state";
import {
	CAMERA_AUDIO_SAMPLE_RATE,
	type CameraAudioRecorder,
	type CameraAudioSnapshot,
	createCameraAudioRecorder,
	encodeCameraWav,
	validateAudioBufferSeconds,
	validateAudioDuration,
} from "./camera-audio";
import {
	CameraError,
	type CameraOverlayUpdate,
	parseCameraOverlayUpdate,
} from "./camera-overlays";

export interface CameraFrame {
	surfaceId: string;
	componentId: string;
	sessionId: string;
	frameId: string;
	capturedAt: string;
	width: number;
	height: number;
	/** Encoded pixels follow the upright video frame, without preview mirroring. */
	mirrored: false;
	orientation: 0;
	image: {
		name: string;
		type: string;
		size: number;
		url: string;
		backendUrl?: string;
		flowPath?: ITemporaryFlowPath;
	};
}

export interface CameraCommandContext {
	appId?: string;
	surfaceId?: string;
	signal?: AbortSignal;
	executionTarget?: ITemporaryUploadExecutionTarget;
}

export interface CameraAudioClip {
	surfaceId: string;
	componentId: string;
	sessionId: string;
	clipId: string;
	capturedAt: string;
	startedAt: string;
	endedAt: string;
	durationMs: number;
	requestedDurationMs: number;
	sampleRate: 16000;
	channels: 1;
	audio: CameraFrame["image"];
}

export interface CameraSessionState {
	status: "stopped" | "starting" | "live" | "frozen" | "error";
	sessionId?: string;
	width: number;
	height: number;
	frameId?: string;
	busy: boolean;
	error?: string;
	overlays?: CameraOverlayUpdate;
	audioEnabled?: boolean;
	audioActive?: boolean;
	audioBufferSeconds?: number;
	audioAvailableMs?: number;
}

interface CameraSessionOptions {
	appId?: string;
	surfaceId: string;
	componentId: string;
	video: () => HTMLVideoElement | null;
	isVisible: () => boolean;
	upload: (
		file: File,
		context?: CameraCommandContext,
	) => Promise<CameraFrame["image"]>;
	onState: (state: CameraSessionState) => void;
	getMedia?: (constraints: MediaStreamConstraints) => Promise<MediaStream>;
	/** Injectable encoder keeps capture and lifecycle races testable without a real device. */
	encode?: (
		video: HTMLVideoElement,
		width: number,
		height: number,
		quality: number,
	) => Promise<Blob>;
	resize?: (
		blob: Blob,
		width: number,
		height: number,
		quality: number,
	) => Promise<Blob>;
	audioRecorder?: typeof createCameraAudioRecorder;
}

const sessions = new Map<string, CameraSession>();
const sessionKey = (
	app: string | undefined,
	surface: string,
	component: string,
) => JSON.stringify([app, surface, component]);
const id = () => globalThis.crypto.randomUUID();

async function encodeFrame(
	video: CanvasImageSource,
	width: number,
	height: number,
	quality: number,
): Promise<Blob> {
	const canvas = document.createElement("canvas");
	canvas.width = width;
	canvas.height = height;
	const context = canvas.getContext("2d");
	if (!context)
		throw new CameraError(
			"unsupported",
			"Image capture is unavailable on this device.",
		);
	context.drawImage(video, 0, 0, width, height);
	return new Promise((resolve, reject) =>
		canvas.toBlob(
			(blob) =>
				blob
					? resolve(blob)
					: reject(
							new CameraError(
								"capture_failed",
								"The current frame could not be encoded.",
							),
						),
			"image/jpeg",
			quality,
		),
	);
}

async function resizeFrozenFrame(
	blob: Blob,
	width: number,
	height: number,
	quality: number,
) {
	if (typeof createImageBitmap === "function") {
		const bitmap = await createImageBitmap(blob);
		try {
			return await encodeFrame(bitmap, width, height, quality);
		} finally {
			bitmap.close();
		}
	}
	const url = URL.createObjectURL(blob);
	try {
		const image = new Image();
		await new Promise<void>((resolve, reject) => {
			image.onload = () => resolve();
			image.onerror = () =>
				reject(
					new CameraError(
						"capture_failed",
						"The frozen frame could not be resized.",
					),
				);
			image.src = url;
		});
		return await encodeFrame(image, width, height, quality);
	} finally {
		URL.revokeObjectURL(url);
	}
}

/** A camera belongs to one mounted surface. No command can start a camera session. */
export class CameraSession {
	state: CameraSessionState = {
		status: "stopped",
		width: 0,
		height: 0,
		busy: false,
	};
	private stream: MediaStream | null = null;
	private revision = 0;
	private disposed = false;
	private capturing = false;
	private overlayTimer?: ReturnType<typeof setTimeout>;
	private sessionAbort = new AbortController();
	private frozenBlob?: Blob;
	private frozenSize?: { width: number; height: number };
	private frozenCapturedAt?: string;
	private audio?: CameraAudioRecorder;

	constructor(readonly options: CameraSessionOptions) {
		const key = sessionKey(
			options.appId,
			options.surfaceId,
			options.componentId,
		);
		sessions.get(key)?.dispose();
		sessions.set(key, this);
	}

	get signal() {
		return this.sessionAbort.signal;
	}

	private publish(patch: Partial<CameraSessionState>) {
		this.state = { ...this.state, ...patch };
		if (!this.disposed) this.options.onState(this.state);
	}

	private available(revision = this.revision, signal?: AbortSignal) {
		if (
			this.disposed ||
			revision !== this.revision ||
			signal?.aborted ||
			!this.options.isVisible()
		)
			throw new CameraError(
				"screen_closed",
				"This camera screen is no longer active.",
			);
	}

	async start(
		constraints: MediaTrackConstraints = {},
		audioEnabled = false,
		audioBufferSeconds = 60,
	) {
		this.available();
		if (this.state.status === "starting" || this.stream) return;
		if (typeof audioEnabled !== "boolean")
			throw new CameraError(
				"invalid_arguments",
				"Microphone audio must be explicitly enabled with a boolean value.",
			);
		validateAudioBufferSeconds(audioBufferSeconds);
		this.sessionAbort = new AbortController();
		const revision = ++this.revision;
		this.publish({
			status: "starting",
			error: undefined,
			frameId: undefined,
			overlays: undefined,
			sessionId: undefined,
			audioEnabled,
			audioActive: false,
			audioBufferSeconds,
			audioAvailableMs: 0,
		});
		try {
			const getMedia =
				this.options.getMedia ??
				((value) => navigator.mediaDevices.getUserMedia(value));
			const media = getMedia({
				video: constraints,
				audio: audioEnabled
					? { channelCount: 1, echoCancellation: true, noiseSuppression: true }
					: false,
			}).then((stream) => {
				try {
					this.available(revision);
				} catch (error) {
					for (const track of stream.getTracks()) track.stop();
					throw error;
				}
				this.stream = stream;
				return stream;
			});
			// Start Web Audio inside the user's gesture, while the media permission is pending.
			const recording = audioEnabled
				? (this.options.audioRecorder ?? createCameraAudioRecorder)(
						media,
						audioBufferSeconds,
						{
							signal: this.sessionAbort.signal,
							onAvailable: (audioAvailableMs) => {
								if (revision === this.revision && this.state.audioActive)
									this.publish({ audioAvailableMs });
							},
							onError: (error) => {
								if (revision === this.revision) this.stop(error.message);
							},
						},
					)
				: Promise.resolve(undefined);
			const [stream, audio] = await Promise.all([media, recording]);
			try {
				this.available(revision);
			} catch (error) {
				audio?.dispose();
				throw error;
			}
			this.audio = audio;
			for (const track of stream.getTracks())
				track.addEventListener(
					"ended",
					() => {
						if (revision === this.revision)
							this.stop("Camera or microphone access ended.");
					},
					{ signal: this.sessionAbort.signal },
				);
			const video = this.options.video();
			if (!video)
				throw new CameraError(
					"screen_closed",
					"The camera preview is no longer available.",
				);
			video.srcObject = stream;
			await video.play();
			this.available(revision);
			this.publish({
				status: "live",
				sessionId: id(),
				width: video.videoWidth,
				height: video.videoHeight,
				audioActive: Boolean(audio),
			});
		} catch (error) {
			if (revision !== this.revision || this.disposed) return;
			this.stop();
			const message =
				error instanceof Error && error.name === "NotAllowedError"
					? "Camera or microphone permission was denied. Allow access in your device or browser settings, then try again."
					: error instanceof Error
						? error.message
						: "The camera could not be started.";
			this.publish({ status: "error", error: message });
			throw new CameraError("camera_unavailable", message);
		}
	}

	stop(reason?: string) {
		++this.revision;
		this.sessionAbort.abort();
		this.audio?.dispose();
		this.audio = undefined;
		clearTimeout(this.overlayTimer);
		const stream = this.stream;
		this.stream = null;
		for (const track of stream?.getTracks() ?? []) track.stop();
		const video = this.options.video();
		if (video) {
			video.pause();
			video.srcObject = null;
		}
		this.frozenBlob = undefined;
		this.frozenSize = undefined;
		this.frozenCapturedAt = undefined;
		this.publish({
			status: "stopped",
			sessionId: undefined,
			frameId: undefined,
			overlays: undefined,
			busy: false,
			error: reason,
			width: 0,
			height: 0,
			audioActive: false,
			audioAvailableMs: 0,
		});
	}

	private async withCapture<T>(
		context: CameraCommandContext | undefined,
		capture: (context: CameraCommandContext) => Promise<T>,
	): Promise<T> {
		this.available(undefined, context?.signal);
		if (this.capturing)
			throw new CameraError(
				"camera_busy",
				"A camera capture is already in progress.",
			);
		const revision = this.revision;
		const operation = new AbortController();
		const signal = AbortSignal.any([
			this.signal,
			operation.signal,
			...(context?.signal ? [context.signal] : []),
		]);
		this.capturing = true;
		this.publish({ busy: true });
		try {
			const result = await capture({ ...context, signal });
			this.available(revision, signal);
			return result;
		} catch (error) {
			// A failed combined capture must cancel its sibling upload before unlocking.
			operation.abort();
			throw error;
		} finally {
			this.capturing = false;
			if (revision === this.revision) this.publish({ busy: false });
		}
	}

	private async snapshotFrame(
		maxWidth: number,
		quality: number,
		capturedAt: number,
	) {
		const sessionId = this.state.sessionId;
		const video = this.options.video();
		if (!sessionId || !video || !["live", "frozen"].includes(this.state.status))
			throw new CameraError(
				"camera_not_started",
				"Start the camera on this screen before capturing a frame.",
			);
		if (
			!Number.isFinite(maxWidth) ||
			maxWidth < 160 ||
			maxWidth > 4096 ||
			!Number.isFinite(quality) ||
			quality < 0.1 ||
			quality > 1
		)
			throw new CameraError(
				"invalid_arguments",
				"Capture width must be 160 to 4096 and quality must be 0.1 to 1.",
			);
		const originalWidth = this.frozenSize?.width ?? video.videoWidth;
		const originalHeight = this.frozenSize?.height ?? video.videoHeight;
		if (!originalWidth || !originalHeight)
			throw new CameraError(
				"frame_not_ready",
				"Wait for the first camera frame.",
			);
		const scale = Math.min(1, maxWidth / originalWidth, 4096 / originalHeight);
		const width = Math.round(originalWidth * scale);
		const height = Math.round(originalHeight * scale);
		const metadata: Omit<CameraFrame, "image"> = {
			surfaceId: this.options.surfaceId,
			componentId: this.options.componentId,
			sessionId,
			frameId: id(),
			capturedAt: this.frozenCapturedAt ?? new Date(capturedAt).toISOString(),
			width,
			height,
			mirrored: false,
			orientation: 0,
		};
		// The encoder draws the current pixels before its asynchronous JPEG conversion.
		const blob = this.frozenBlob
			? width === originalWidth && height === originalHeight
				? this.frozenBlob
				: await (this.options.resize ?? resizeFrozenFrame)(
						this.frozenBlob,
						width,
						height,
						quality,
					)
			: await (this.options.encode ?? encodeFrame)(
					video,
					width,
					height,
					quality,
				);
		return { metadata, blob, originalWidth, originalHeight };
	}

	private async uploadFrame(
		snapshot: Awaited<ReturnType<CameraSession["snapshotFrame"]>>,
		context: CameraCommandContext,
	): Promise<CameraFrame> {
		this.available(undefined, context.signal);
		const image = await this.options.upload(
			new File([snapshot.blob], `camera-${snapshot.metadata.frameId}.jpg`, {
				type: "image/jpeg",
			}),
			context,
		);
		this.available(undefined, context.signal);
		return { ...snapshot.metadata, image };
	}

	private commitFrame(
		snapshot: Awaited<ReturnType<CameraSession["snapshotFrame"]>>,
		context: CameraCommandContext,
	) {
		this.available(undefined, context.signal);
		this.publish({
			frameId: snapshot.metadata.frameId,
			width: snapshot.originalWidth,
			height: snapshot.originalHeight,
			overlays: undefined,
		});
	}

	private snapshotAudio(durationMs: number, capturedAt: number) {
		validateAudioDuration(durationMs);
		if (!this.state.audioEnabled)
			throw new CameraError(
				"audio_disabled",
				"Enable microphone audio on Camera View, then press Start.",
			);
		if (
			!this.audio ||
			!this.state.audioActive ||
			this.state.status !== "live" ||
			!this.state.sessionId
		)
			throw new CameraError(
				"audio_not_live",
				"Resume or start the camera before capturing microphone audio.",
			);
		return this.audio.snapshot(durationMs, capturedAt);
	}

	private async uploadAudio(
		snapshot: CameraAudioSnapshot,
		durationMs: number,
		capturedAt: number,
		context: CameraCommandContext,
	): Promise<CameraAudioClip> {
		this.available(undefined, context.signal);
		const sessionId = this.state.sessionId;
		if (!sessionId)
			throw new CameraError(
				"audio_not_live",
				"Microphone recording is no longer active.",
			);
		const clipId = id();
		const audio = await this.options.upload(
			new File([encodeCameraWav(snapshot.pcm)], `camera-audio-${clipId}.wav`, {
				type: "audio/wav",
			}),
			context,
		);
		this.available(undefined, context.signal);
		return {
			surfaceId: this.options.surfaceId,
			componentId: this.options.componentId,
			sessionId,
			clipId,
			capturedAt: new Date(capturedAt).toISOString(),
			startedAt: new Date(snapshot.startedAt).toISOString(),
			endedAt: new Date(snapshot.endedAt).toISOString(),
			durationMs: (snapshot.pcm.length / CAMERA_AUDIO_SAMPLE_RATE) * 1000,
			requestedDurationMs: durationMs,
			sampleRate: CAMERA_AUDIO_SAMPLE_RATE,
			channels: 1,
			audio,
		};
	}

	capture(
		context?: CameraCommandContext,
		maxWidth = 1280,
		quality = 0.85,
	): Promise<CameraFrame> {
		return this.withCapture(context, async (live) => {
			const snapshot = await this.snapshotFrame(maxWidth, quality, Date.now());
			const frame = await this.uploadFrame(snapshot, live);
			this.commitFrame(snapshot, live);
			return frame;
		});
	}

	captureAudio(
		context?: CameraCommandContext,
		durationMs = 10_000,
	): Promise<CameraAudioClip> {
		return this.withCapture(context, async (live) => {
			const capturedAt = Date.now();
			const snapshot = await this.snapshotAudio(durationMs, capturedAt);
			return this.uploadAudio(snapshot, durationMs, capturedAt, live);
		});
	}

	captureInput(
		context?: CameraCommandContext,
		durationMs = 10_000,
		maxWidth = 1280,
		quality = 0.85,
	): Promise<{ frame: CameraFrame; audio: CameraAudioClip }> {
		return this.withCapture(context, async (live) => {
			const capturedAt = Date.now();
			// Both sensors take their snapshot now, before either encoding or upload completes.
			const audio = this.snapshotAudio(durationMs, capturedAt);
			const frame = this.snapshotFrame(maxWidth, quality, capturedAt);
			const [audioSnapshot, frameSnapshot] = await Promise.all([audio, frame]);
			this.available(undefined, live.signal);
			const [audioClip, cameraFrame] = await Promise.all([
				this.uploadAudio(audioSnapshot, durationMs, capturedAt, live),
				this.uploadFrame(frameSnapshot, live),
			]);
			this.commitFrame(frameSnapshot, live);
			return { frame: cameraFrame, audio: audioClip };
		});
	}

	async freeze(signal?: AbortSignal) {
		this.available(undefined, signal);
		if (this.capturing)
			throw new CameraError(
				"camera_busy",
				"Wait for the current camera capture to finish.",
			);
		if (this.state.status === "frozen") return;
		if (this.state.status !== "live")
			throw new CameraError(
				"camera_not_started",
				"Start the camera before freezing it.",
			);
		const revision = this.revision;
		const video = this.options.video();
		if (!video?.videoWidth)
			throw new CameraError(
				"frame_not_ready",
				"Wait for the first camera frame.",
			);
		video.pause();
		this.audio?.pause();
		for (const track of this.stream?.getAudioTracks?.() ?? [])
			track.enabled = false;
		this.capturing = true;
		this.publish({ busy: true, audioActive: false, audioAvailableMs: 0 });
		try {
			const capturedAt = new Date().toISOString();
			const scale = Math.min(
				1,
				1280 / video.videoWidth,
				4096 / video.videoHeight,
			);
			const size = {
				width: Math.round(video.videoWidth * scale),
				height: Math.round(video.videoHeight * scale),
			};
			const blob = await (this.options.encode ?? encodeFrame)(
				video,
				size.width,
				size.height,
				0.9,
			);
			this.available(revision, signal);
			this.frozenBlob = blob;
			this.frozenSize = size;
			this.frozenCapturedAt = capturedAt;
			this.publish({
				status: "frozen",
				frameId: undefined,
				overlays: undefined,
			});
		} catch (error) {
			if (revision === this.revision) {
				void video.play();
				this.audio?.resume();
				for (const track of this.stream?.getAudioTracks?.() ?? [])
					track.enabled = true;
				this.publish({ audioActive: Boolean(this.audio), audioAvailableMs: 0 });
			}
			throw error;
		} finally {
			this.capturing = false;
			if (revision === this.revision) this.publish({ busy: false });
		}
	}

	async resume(signal?: AbortSignal) {
		this.available(undefined, signal);
		if (this.state.status === "live") return;
		if (this.capturing)
			throw new CameraError(
				"camera_busy",
				"Wait for the current camera capture to finish.",
			);
		if (!this.stream)
			throw new CameraError(
				"camera_not_started",
				"Start the camera on this screen first.",
			);
		const revision = this.revision;
		await this.options.video()?.play();
		this.available(revision, signal);
		this.audio?.resume();
		for (const track of this.stream.getAudioTracks?.() ?? [])
			track.enabled = true;
		this.frozenBlob = undefined;
		this.frozenSize = undefined;
		this.frozenCapturedAt = undefined;
		this.publish({
			status: "live",
			overlays: undefined,
			frameId: undefined,
			audioActive: Boolean(this.audio),
			audioAvailableMs: 0,
		});
	}

	updateOverlays(value: unknown) {
		this.available();
		const update = parseCameraOverlayUpdate(value);
		if (
			update.sessionId !== this.state.sessionId ||
			update.frameId !== this.state.frameId
		)
			throw new CameraError(
				"stale_frame",
				"These overlays belong to an older camera frame or session.",
			);
		clearTimeout(this.overlayTimer);
		this.publish({ overlays: update });
		this.overlayTimer = setTimeout(
			() => this.publish({ overlays: undefined }),
			update.ttlMs,
		);
	}

	dispose() {
		if (this.disposed) return;
		this.stop();
		this.disposed = true;
		const key = sessionKey(
			this.options.appId,
			this.options.surfaceId,
			this.options.componentId,
		);
		if (sessions.get(key) === this) sessions.delete(key);
	}
}

export async function executeCameraCommand(
	command: string,
	args: unknown,
	context: CameraCommandContext,
): Promise<unknown> {
	if (!args || typeof args !== "object" || Array.isArray(args))
		throw new CameraError(
			"invalid_arguments",
			"Camera command arguments must be an object.",
		);
	const value = args as Record<string, unknown>;
	if (
		typeof value.surfaceId !== "string" ||
		typeof value.componentId !== "string"
	)
		throw new CameraError(
			"invalid_arguments",
			"Camera commands require surfaceId and componentId.",
		);
	const session = sessions.get(
		sessionKey(context.appId, value.surfaceId, value.componentId),
	);
	if (
		!session ||
		!context.appId ||
		context.appId !== session.options.appId ||
		(context.surfaceId && context.surfaceId !== value.surfaceId)
	)
		throw new CameraError(
			"screen_closed",
			"This camera is not available to the requesting screen.",
		);
	if (context.signal?.aborted)
		throw new CameraError("cancelled", "The requesting Event was cancelled.");
	if (!value.sessionId || value.sessionId !== session.state.sessionId)
		throw new CameraError(
			"stale_session",
			"This command needs the current camera session from this screen.",
		);
	switch (command) {
		case "camera.capture":
			return session.capture(
				context,
				value.maxWidth === undefined ? 1280 : Number(value.maxWidth),
				value.quality === undefined ? 0.85 : Number(value.quality),
			);
		case "camera.captureAudio":
			return session.captureAudio(
				context,
				value.durationMs === undefined ? 10_000 : Number(value.durationMs),
			);
		case "camera.captureInput":
			return session.captureInput(
				context,
				value.durationMs === undefined ? 10_000 : Number(value.durationMs),
				value.maxWidth === undefined ? 1280 : Number(value.maxWidth),
				value.quality === undefined ? 0.85 : Number(value.quality),
			);
		case "camera.freeze":
			await session.freeze(context.signal);
			break;
		case "camera.resume":
			await session.resume(context.signal);
			break;
		case "camera.stop":
			session.stop();
			break;
		case "camera.updateOverlays":
			session.updateOverlays(value);
			break;
		case "camera.clearOverlays":
			session.updateOverlays({ ...value, overlays: [] });
			break;
		default:
			throw new CameraError("unsupported", "Unknown camera command.");
	}
	return { status: session.state.status, sessionId: session.state.sessionId };
}

/** Native hosts dispatch this on lock/background in addition to browser visibility events. */
export function stopAllCameraSessions() {
	for (const session of sessions.values()) session.stop();
}
