import { afterEach, describe, expect, mock, test } from "bun:test";
import {
	type CameraAudioRecorder,
	type CameraAudioSnapshot,
	createCameraPcmRing,
} from "./camera-audio";
import {
	cameraImageRect,
	cameraOverlayX,
	parseCameraOverlayUpdate,
} from "./camera-overlays";
import { CameraSession, executeCameraCommand } from "./camera-session";

const instances: CameraSession[] = [];
afterEach(() => {
	for (const session of instances.splice(0)) session.dispose();
});

function fixture(
	overrides: Partial<ConstructorParameters<typeof CameraSession>[0]> = {},
) {
	const track = Object.assign(new EventTarget(), { stop: mock(() => {}) });
	const stream = { getTracks: () => [track] } as unknown as MediaStream;
	const video = {
		videoWidth: 1920,
		videoHeight: 1080,
		play: mock(async () => {}),
		pause: mock(() => {}),
		srcObject: null,
	} as unknown as HTMLVideoElement;
	const upload = mock(async (file: File) => ({
		name: file.name,
		type: file.type,
		size: file.size,
		url: "temporary://camera",
	}));
	const session = new CameraSession({
		appId: "app",
		surfaceId: "page",
		componentId: "camera",
		video: () => video,
		isVisible: () => true,
		getMedia: mock(async () => stream),
		encode: mock(async () => new Blob(["jpeg"], { type: "image/jpeg" })),
		upload,
		onState: () => {},
		...overrides,
	});
	instances.push(session);
	return { session, track, video, upload, stream };
}

describe("camera session lifecycle", () => {
	test("an ended notification from an older stream cannot stop its replacement", async () => {
		const oldTrack = Object.assign(new EventTarget(), { stop: mock(() => {}) });
		const newTrack = Object.assign(new EventTarget(), { stop: mock(() => {}) });
		let calls = 0;
		const { session } = fixture({
			getMedia: async () => {
				const track = calls++ === 0 ? oldTrack : newTrack;
				return { getTracks: () => [track] } as unknown as MediaStream;
			},
		});
		await session.start();
		session.stop();
		await session.start();
		oldTrack.dispatchEvent(new Event("ended"));
		expect(session.state.status).toBe("live");
		newTrack.dispatchEvent(new Event("ended"));
		expect(session.state.status).toBe("stopped");
	});
	test("a camera permission reply arriving after navigation is stopped immediately", async () => {
		let resolve!: (stream: MediaStream) => void;
		const media = new Promise<MediaStream>((done) => {
			resolve = done;
		});
		const { session, stream, track, video } = fixture({
			getMedia: () => media,
		});
		const starting = session.start();
		session.dispose();
		resolve(stream);
		await starting;
		expect(track.stop).toHaveBeenCalledTimes(1);
		expect(video.srcObject).toBeNull();
		expect(session.state.sessionId).toBeUndefined();
	});

	test("closing the page during encoding prevents any upload", async () => {
		let resolve!: (blob: Blob) => void;
		const encoding = new Promise<Blob>((done) => {
			resolve = done;
		});
		const { session, upload, track } = fixture({ encode: () => encoding });
		await session.start();
		const captured = session.capture();
		session.stop();
		resolve(new Blob(["jpeg"]));
		await expect(captured).rejects.toMatchObject({ code: "screen_closed" });
		expect(upload).not.toHaveBeenCalled();
		expect(track.stop).toHaveBeenCalledTimes(1);
	});

	test("visibility is checked again before replying and overlapping capture is rejected", async () => {
		let visible = true;
		let resolve!: (value: {
			name: string;
			type: string;
			size: number;
			url: string;
		}) => void;
		const upload = new Promise<{
			name: string;
			type: string;
			size: number;
			url: string;
		}>((done) => {
			resolve = done;
		});
		const { session } = fixture({
			isVisible: () => visible,
			upload: () => upload,
		});
		await session.start();
		const captured = session.capture();
		await expect(session.capture()).rejects.toMatchObject({
			code: "camera_busy",
		});
		await Promise.resolve();
		visible = false;
		resolve({
			name: "frame.jpg",
			type: "image/jpeg",
			size: 4,
			url: "temp://frame",
		});
		await expect(captured).rejects.toMatchObject({ code: "screen_closed" });
		expect(session.state.frameId).toBeUndefined();
	});

	test("device commands cannot start cameras or cross apps or surfaces", async () => {
		const { session } = fixture();
		const args = { surfaceId: "page", componentId: "camera" };
		await expect(
			executeCameraCommand("camera.capture", args, { appId: "app" }),
		).rejects.toMatchObject({ code: "stale_session" });
		await session.start();
		await expect(
			executeCameraCommand("camera.capture", args, { appId: "app" }),
		).rejects.toMatchObject({ code: "stale_session" });
		await expect(
			executeCameraCommand("camera.capture", args, { appId: "other" }),
		).rejects.toMatchObject({ code: "screen_closed" });
		await expect(
			executeCameraCommand("camera.capture", args, {
				appId: "app",
				surfaceId: "other",
			}),
		).rejects.toMatchObject({ code: "screen_closed" });
		await expect(
			executeCameraCommand(
				"camera.start",
				{ ...args, sessionId: session.state.sessionId },
				{ appId: "app" },
			),
		).rejects.toMatchObject({ code: "unsupported" });
	});

	test("a cancelled requesting Event cannot capture", async () => {
		const { session, upload } = fixture();
		await session.start();
		const controller = new AbortController();
		controller.abort();
		await expect(
			executeCameraCommand(
				"camera.capture",
				{ surfaceId: "page", componentId: "camera" },
				{ appId: "app", signal: controller.signal },
			),
		).rejects.toMatchObject({ code: "cancelled" });
		expect(upload).not.toHaveBeenCalled();
	});

	test("old frames and ended sessions cannot apply overlays or stop a new camera", async () => {
		const { session } = fixture();
		await session.start();
		const first = await session.capture();
		session.updateOverlays({
			...first,
			overlays: [{ id: "word", type: "text", x: 0.1, y: 0.2, text: "Hello" }],
		});
		expect(session.state.overlays?.overlays[0].text).toBe("Hello");
		const second = await session.capture();
		expect(second.frameId).not.toBe(first.frameId);
		expect(() => session.updateOverlays({ ...first, overlays: [] })).toThrow(
			"older camera frame",
		);
		session.stop();
		await session.start();
		await expect(
			executeCameraCommand("camera.stop", first, { appId: "app" }),
		).rejects.toMatchObject({ code: "stale_session" });
		await expect(
			executeCameraCommand("camera.capture", first, { appId: "app" }),
		).rejects.toMatchObject({ code: "stale_session" });
		expect(session.state.status).toBe("live");
	});

	test("freeze captures stable pixels and resume invalidates their annotations", async () => {
		const encode = mock(async () => new Blob(["same frozen pixels"]));
		const { session, video } = fixture({ encode });
		await session.start();
		await session.freeze();
		const frame = await session.capture();
		const nextFrame = await session.capture();
		expect(nextFrame.capturedAt).toBe(frame.capturedAt);
		expect(encode).toHaveBeenCalledTimes(1);
		expect(frame.width).toBe(1280);
		expect(frame.height).toBe(720);
		expect(frame.mirrored).toBe(false);
		expect(video.pause).toHaveBeenCalled();
		await session.resume();
		expect(session.state.frameId).toBeUndefined();
	});

	test("freezing excludes concurrent captures and aborting discards the frozen result", async () => {
		let resolve!: (blob: Blob) => void;
		const encoding = new Promise<Blob>((done) => {
			resolve = done;
		});
		const { session, upload } = fixture({ encode: () => encoding });
		await session.start();
		const controller = new AbortController();
		const freezing = session.freeze(controller.signal);
		await expect(session.capture()).rejects.toMatchObject({
			code: "camera_busy",
		});
		controller.abort();
		resolve(new Blob(["jpeg"]));
		await expect(freezing).rejects.toMatchObject({ code: "screen_closed" });
		expect(session.state.status).toBe("live");
		expect(session.state.busy).toBe(false);
		expect(upload).not.toHaveBeenCalled();
	});
});

test("frozen captures honor a smaller requested width without reading live pixels", async () => {
	const encode = mock(async () => new Blob(["frozen"]));
	const resize = mock(
		async (_blob: Blob, _width: number, _height: number, _quality: number) =>
			new Blob(["smaller"]),
	);
	const { session } = fixture({ encode, resize });
	await session.start();
	await session.freeze();
	const frame = await session.capture(undefined, 640, 0.5);
	expect(frame.width).toBe(640);
	expect(frame.height).toBe(360);
	expect(encode).toHaveBeenCalledTimes(1);
	expect(resize.mock.calls[0]?.slice(1)).toEqual([640, 360, 0.5]);
});

describe("camera overlay contract", () => {
	test("contain, cover and mirrored coordinates remain aligned", () => {
		expect(
			cameraImageRect(
				{ width: 400, height: 400 },
				{ width: 1920, height: 1080 },
			),
		).toEqual({ x: 0, y: 87.5, width: 400, height: 225 });
		const cover = cameraImageRect(
			{ width: 400, height: 400 },
			{ width: 1920, height: 1080 },
			"cover",
		);
		expect(cover.height).toBe(400);
		expect(cover.x).toBeCloseTo(-155.5556);
		expect(cameraOverlayX(0.1, 0.2, true)).toBeCloseTo(0.7);
	});
	test("malformed geometry, duplicate ids and CSS resource URLs are rejected", () => {
		const box = {
			id: "box",
			type: "box",
			x: 0.2,
			y: 0.3,
			width: 0.2,
			height: 0.2,
		};
		const update = (overlays: unknown[]) => ({
			sessionId: "s",
			frameId: "f",
			overlays,
		});
		expect(() =>
			parseCameraOverlayUpdate(update([{ ...box, x: Number.NaN }])),
		).toThrow();
		expect(() =>
			parseCameraOverlayUpdate(update([{ ...box, width: 0.9 }])),
		).toThrow();
		expect(() => parseCameraOverlayUpdate(update([box, box]))).toThrow();
		expect(() =>
			parseCameraOverlayUpdate(
				update([{ ...box, color: "url(https://example.com)" }]),
			),
		).toThrow();
		expect(() =>
			parseCameraOverlayUpdate({ ...update([box]), ttlMs: 999999 }),
		).toThrow();
	});
});

function audioFixture(
	overrides: Partial<ConstructorParameters<typeof CameraSession>[0]> = {},
) {
	const microphone = Object.assign(new EventTarget(), {
		enabled: true,
		stop: mock(() => {}),
	});
	const camera = Object.assign(new EventTarget(), { stop: mock(() => {}) });
	const stream = {
		getTracks: () => [camera, microphone],
		getAudioTracks: () => [microphone],
	} as unknown as MediaStream;
	const ring = createCameraPcmRing(16_000, 60);
	let seconds = 0;
	const recorder: CameraAudioRecorder = {
		snapshot: mock(async (durationMs, capturedAt) => {
			const { pcm } = ring.snapshot(durationMs, seconds);
			if (!pcm.length) throw new Error("Wait for microphone samples.");
			return {
				pcm,
				endedAt: capturedAt,
				startedAt: capturedAt - pcm.length / 16,
			};
		}),
		pause: mock(() => ring.clear()),
		resume: mock(() => {
			ring.clear();
			seconds = 0;
		}),
		dispose: mock(() => ring.clear()),
	};
	const media = mock(async (_constraints: MediaStreamConstraints) => stream);
	const factory = mock(async () => recorder);
	const result = fixture({
		getMedia: media,
		audioRecorder: factory,
		...overrides,
	});
	return {
		...result,
		microphone,
		camera,
		recorder,
		factory,
		media,
		ring,
		record(duration: number) {
			ring.append([new Float32Array(duration * 16_000).fill(0.5)], seconds);
			seconds += duration;
		},
	};
}

describe("camera microphone commands", () => {
	test("microphone acquisition is opt-in and commands cannot enable it", async () => {
		const { session, factory, media } = audioFixture();
		await session.start();
		expect(media.mock.calls[0]?.[0]).toMatchObject({ audio: false });
		expect(factory).not.toHaveBeenCalled();
		await expect(session.captureAudio()).rejects.toMatchObject({
			code: "audio_disabled",
		});
		session.stop();
		await session.start({}, true, 60);
		expect(media.mock.calls[1]?.[0]).toMatchObject({
			audio: { channelCount: 1 },
		});
		expect(factory).toHaveBeenCalledTimes(1);
		expect(session.state.audioActive).toBe(true);
	});

	test("a shorter available recording exports complete WAV metadata through the execution target", async () => {
		const { session, record, upload } = audioFixture();
		await session.start({}, true);
		record(5);
		const clip = (await executeCameraCommand(
			"camera.captureAudio",
			{
				surfaceId: "page",
				componentId: "camera",
				sessionId: session.state.sessionId,
				durationMs: 10_000,
			},
			{ appId: "app", executionTarget: "remote" },
		)) as Awaited<ReturnType<CameraSession["captureAudio"]>>;
		expect(clip).toMatchObject({
			sessionId: session.state.sessionId,
			requestedDurationMs: 10000,
			durationMs: 5000,
			sampleRate: 16000,
			channels: 1,
		});
		expect(Date.parse(clip.endedAt) - Date.parse(clip.startedAt)).toBe(5000);
		expect(clip.audio.size).toBe(160_044);
		expect(clip.audio.type).toBe("audio/wav");
		const file = upload.mock.calls[0]?.[0];
		expect(
			new TextDecoder().decode((await file?.arrayBuffer())?.slice(0, 4)),
		).toBe("RIFF");
	});

	test("freeze disables granted microphone tracks and clears history; resume reuses them", async () => {
		const { session, record, microphone, media, recorder, ring } =
			audioFixture();
		await session.start({}, true);
		record(5);
		await session.freeze();
		expect(microphone.enabled).toBe(false);
		expect(microphone.stop).not.toHaveBeenCalled();
		expect(session.state.audioActive).toBe(false);
		expect(ring.availableMs()).toBe(0);
		await expect(session.captureAudio()).rejects.toMatchObject({
			code: "audio_not_live",
		});
		await expect(session.captureInput()).rejects.toMatchObject({
			code: "audio_not_live",
		});
		await session.resume();
		expect(recorder.resume).toHaveBeenCalledTimes(1);
		expect(microphone.enabled).toBe(true);
		expect(media).toHaveBeenCalledTimes(1);
		expect(session.state.audioAvailableMs).toBe(0);
		record(0.5);
		expect((await session.captureAudio()).durationMs).toBe(500);
		session.stop();
		expect(microphone.stop).toHaveBeenCalledTimes(1);
		expect(recorder.dispose).toHaveBeenCalledTimes(1);
		expect(ring.availableMs()).toBe(0);
	});

	test("resuming a live session preserves accumulated microphone history", async () => {
		const { session, record, recorder, media } = audioFixture();
		await session.start({}, true);
		record(5);
		await session.resume();
		expect(recorder.resume).not.toHaveBeenCalled();
		expect(media).toHaveBeenCalledTimes(1);
		expect((await session.captureAudio()).durationMs).toBe(5000);
	});

	test("combined capture snapshots both sensors before waiting for encoding or uploads", async () => {
		let finishFrame!: (value: Blob) => void;
		let finishAudio!: (value: CameraAudioSnapshot) => void;
		const frame = new Promise<Blob>((resolve) => {
			finishFrame = resolve;
		});
		const audio = new Promise<CameraAudioSnapshot>((resolve) => {
			finishAudio = resolve;
		});
		const snapshot = mock((_durationMs: number, _capturedAt: number) => audio);
		const encode = mock(() => frame);
		const { session, upload } = audioFixture({
			encode,
			audioRecorder: async () => ({
				snapshot,
				pause() {},
				resume() {},
				dispose() {},
			}),
		});
		await session.start({}, true);
		const captured = session.captureInput(undefined, 1000);
		expect(snapshot).toHaveBeenCalledTimes(1);
		expect(encode).toHaveBeenCalledTimes(1);
		expect(upload).not.toHaveBeenCalled();
		finishFrame(new Blob(["jpeg"]));
		await Promise.resolve();
		expect(upload).not.toHaveBeenCalled();
		const capturedAt = snapshot.mock.calls[0]?.[1] ?? 0;
		finishAudio({
			pcm: new Float32Array(16_000),
			startedAt: capturedAt - 1000,
			endedAt: capturedAt,
		});
		const result = await captured;
		expect(result.frame.capturedAt).toBe(result.audio.capturedAt);
		expect(result.frame.sessionId).toBe(result.audio.sessionId);
		expect(upload).toHaveBeenCalledTimes(2);
	});

	test("a failed combined upload aborts its sibling and cannot overwrite a newer frame", async () => {
		let finishOld!: (value: {
			name: string;
			type: string;
			size: number;
			url: string;
		}) => void;
		const oldUpload = new Promise<{
			name: string;
			type: string;
			size: number;
			url: string;
		}>((resolve) => {
			finishOld = resolve;
		});
		let siblingSignal: AbortSignal | undefined;
		let imageCount = 0;
		const { session, record } = audioFixture({
			upload: async (file, context) => {
				if (file.type === "audio/wav") throw new Error("WAV upload failed");
				if (imageCount++ === 0) {
					siblingSignal = context?.signal;
					return oldUpload;
				}
				return {
					name: file.name,
					type: file.type,
					size: file.size,
					url: "temporary://new-frame",
				};
			},
		});
		await session.start({}, true);
		record(1);
		await expect(session.captureInput()).rejects.toThrow("WAV upload failed");
		expect(siblingSignal?.aborted).toBe(true);
		expect(session.state.frameId).toBeUndefined();
		const newFrame = await session.capture();
		finishOld({
			name: "old.jpg",
			type: "image/jpeg",
			size: 4,
			url: "temporary://old-frame",
		});
		await Promise.resolve();
		await Promise.resolve();
		expect(session.state.frameId).toBe(newFrame.frameId);
	});

	test("screen closure during audio snapshot prevents upload and aborts a pending upload", async () => {
		let finish!: (value: CameraAudioSnapshot) => void;
		const snapshot = new Promise<CameraAudioSnapshot>((resolve) => {
			finish = resolve;
		});
		const { session, upload } = audioFixture({
			audioRecorder: async () => ({
				snapshot: () => snapshot,
				pause() {},
				resume() {},
				dispose() {},
			}),
		});
		await session.start({}, true);
		const captured = session.captureAudio();
		session.dispose();
		finish({
			pcm: new Float32Array(1600),
			startedAt: Date.now() - 100,
			endedAt: Date.now(),
		});
		await expect(captured).rejects.toMatchObject({ code: "screen_closed" });
		expect(upload).not.toHaveBeenCalled();

		let finishUpload!: (value: {
			name: string;
			type: string;
			size: number;
			url: string;
		}) => void;
		let uploadSignal: AbortSignal | undefined;
		const waitingUpload = new Promise<{
			name: string;
			type: string;
			size: number;
			url: string;
		}>((resolve) => {
			finishUpload = resolve;
		});
		const next = audioFixture({
			upload: (_file, context) => {
				uploadSignal = context?.signal;
				return waitingUpload;
			},
		});
		await next.session.start({}, true);
		next.record(1);
		const controller = new AbortController();
		const request = next.session.captureAudio({ signal: controller.signal });
		await Promise.resolve();
		controller.abort();
		expect(uploadSignal?.aborted).toBe(true);
		finishUpload({
			name: "clip.wav",
			type: "audio/wav",
			size: 32044,
			url: "https://example.com/clip",
		});
		await expect(request).rejects.toMatchObject({ code: "screen_closed" });
	});

	test("audio commands retain app, screen, session and cancellation boundaries", async () => {
		const { session, record, upload } = audioFixture();
		await session.start({}, true);
		record(1);
		const args = {
			surfaceId: "page",
			componentId: "camera",
			sessionId: session.state.sessionId,
		};
		await expect(
			executeCameraCommand("camera.captureAudio", args, { appId: "other" }),
		).rejects.toMatchObject({ code: "screen_closed" });
		await expect(
			executeCameraCommand("camera.captureInput", args, {
				appId: "app",
				surfaceId: "other",
			}),
		).rejects.toMatchObject({ code: "screen_closed" });
		await expect(
			executeCameraCommand(
				"camera.captureAudio",
				{ ...args, sessionId: undefined },
				{ appId: "app" },
			),
		).rejects.toMatchObject({ code: "stale_session" });
		const aborted = new AbortController();
		aborted.abort();
		await expect(
			executeCameraCommand("camera.captureInput", args, {
				appId: "app",
				signal: aborted.signal,
			}),
		).rejects.toMatchObject({ code: "cancelled" });
		session.stop();
		await session.start({}, true);
		await expect(
			executeCameraCommand("camera.captureAudio", args, { appId: "app" }),
		).rejects.toMatchObject({ code: "stale_session" });
		expect(upload).not.toHaveBeenCalled();
	});
});
