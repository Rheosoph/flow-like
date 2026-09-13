import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { CameraViewComponent } from "../types";

let runtime = true;
let pathname = "/camera";
const setBound = mock((_path: string, _value: unknown) => {});
const upload = mock(
	async (
		_file: File,
		_offline?: boolean,
		_appId?: string,
		_target?: string,
	) => ({ url: "https://storage.example/camera-frame" }),
);
const batchUpload = mock(
	async (
		_files: File[],
		_options?: { executionTarget?: string; signal?: AbortSignal },
	) =>
		[] as {
			uploaded?: {
				url: string;
				flowPath?: { path: string; store_ref: string };
			};
			error?: string;
		}[],
);
let useBatchUpload = false;
const target = mock(async () => "remote");
const trigger = mock(
	async (_name: string, _component: unknown, _context: unknown) => {},
);
const resolve = (value: Record<string, unknown>) =>
	value.literalString ??
	value.literalNumber ??
	value.literalBool ??
	(value.literalJson ? JSON.parse(String(value.literalJson)) : undefined);
mock.module("next/navigation", () => ({ usePathname: () => pathname }));
mock.module("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
mock.module("../../../state/backend-state", () => ({
	useBackend: () => ({
		helperState: {
			fileToTemporaryFile: upload,
			filesToTemporaryFiles: useBatchUpload ? batchUpload : undefined,
		},
	}),
}));
mock.module("../ActionHandler", () => ({
	useActionContext: () => ({
		appId: "app",
		isPreviewMode: runtime,
		resolveTemporaryUploadTarget: target,
	}),
	useComponentEventTrigger: () => trigger,
}));
mock.module("../DataContext", () => ({
	useData: () => ({ resolve, setByPath: setBound }),
}));
mock.module("../StyleResolver", () => ({
	resolveStyle: () => "",
	resolveInlineStyle: () => ({}),
}));
mock.module("../../ui/button", () => ({
	Button: ({ children, size: _size, variant: _variant, ...props }: any) => (
		<button {...props}>{children}</button>
	),
}));

let browser: Window;
const createObjectUrl = URL.createObjectURL;
const revokeObjectUrl = URL.revokeObjectURL;
let root: Root;
let stop: ReturnType<typeof mock>;
let getMedia: ReturnType<typeof mock>;
let microphone: {
	enabled: boolean;
	stop: ReturnType<typeof mock>;
	addEventListener: () => void;
};
let audioClosed: ReturnType<typeof mock>;
let audioContext: EventTarget & { state: string };
const defaults: CameraViewComponent = {
	id: "camera",
	type: "cameraView",
	value: { path: "/camera" },
	intervalMs: { literalNumber: 250 },
	eventHandlers: {
		frame: [{ name: "workflow_event", context: { eventId: "capture-event" } }],
	},
};

beforeEach(() => {
	URL.createObjectURL = () => "blob:camera-worklet-test";
	URL.revokeObjectURL = () => {};
	browser = new Window({ url: "https://flow-like.test/camera" });
	Object.assign(browser, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		window: browser,
		document: browser.document,
		navigator: browser.navigator,
		HTMLElement: browser.HTMLElement,
		Element: browser.Element,
		Node: browser.Node,
		MutationObserver: browser.MutationObserver,
		getComputedStyle: browser.getComputedStyle.bind(browser),
		File: browser.File,
		Blob: browser.Blob,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	Object.defineProperty(globalThis, "IntersectionObserver", {
		configurable: true,
		value: undefined,
	});
	Object.defineProperty(globalThis, "ResizeObserver", {
		configurable: true,
		value: undefined,
	});
	runtime = true;
	useBatchUpload = false;
	batchUpload.mockReset();
	pathname = "/camera";
	stop = mock(() => {});
	const track = { stop, addEventListener: () => {} };
	microphone = {
		enabled: true,
		stop: mock(() => {}),
		addEventListener: () => {},
	};
	getMedia = mock(async (constraints: MediaStreamConstraints) => ({
		getTracks: () => (constraints.audio ? [track, microphone] : [track]),
		getAudioTracks: () => (constraints.audio ? [microphone] : []),
	}));
	audioClosed = mock(async () => {});
	Object.assign(globalThis, {
		AudioContext: class extends EventTarget {
			state = "running";
			constructor() {
				super();
				audioContext = this;
			}
			currentTime = 5;
			destination = {};
			audioWorklet = { addModule: async () => {} };
			resume = async () => {};
			close = audioClosed;
			createMediaStreamSource = () => ({
				connect: () => {},
				disconnect: () => {},
			});
		},
		AudioWorkletNode: class {
			epoch = 0;
			port = {
				onmessage: (_event: { data: unknown }) => {},
				close: () => {},
				postMessage: (data: {
					type: string;
					id?: number;
					epoch?: number;
					durationMs?: number;
				}) => {
					if (data.epoch !== undefined) this.epoch = data.epoch;
					if (data.type === "snapshot")
						queueMicrotask(() =>
							this.port.onmessage({
								data: {
									id: data.id,
									epoch: this.epoch,
									pcm: new Float32Array(
										Math.min(5000, data.durationMs ?? 0) * 16,
									),
									endTime: 5,
								},
							}),
						);
				},
			};
			connect = () => {};
			disconnect = () => {};
		},
	});
	Object.defineProperty(browser.navigator, "mediaDevices", {
		configurable: true,
		value: { getUserMedia: getMedia },
	});
	Object.defineProperty(browser.HTMLMediaElement.prototype, "play", {
		configurable: true,
		value: async () => {},
	});
	Object.defineProperty(browser.HTMLMediaElement.prototype, "pause", {
		configurable: true,
		value: () => {},
	});
	const streams = new WeakMap<object, unknown>();
	Object.defineProperty(browser.HTMLMediaElement.prototype, "srcObject", {
		configurable: true,
		get() {
			return streams.get(this);
		},
		set(value) {
			streams.set(this, value);
		},
	});
	Object.defineProperty(browser.HTMLVideoElement.prototype, "videoWidth", {
		configurable: true,
		get: () => 1920,
	});
	Object.defineProperty(browser.HTMLVideoElement.prototype, "videoHeight", {
		configurable: true,
		get: () => 1080,
	});
	Object.defineProperty(browser.HTMLCanvasElement.prototype, "getContext", {
		configurable: true,
		value: () => ({ drawImage: () => {} }),
	});
	Object.defineProperty(browser.HTMLCanvasElement.prototype, "toBlob", {
		configurable: true,
		value: (callback: (blob: Blob) => void) =>
			callback(new Blob(["jpeg"], { type: "image/jpeg" })),
	});
	upload.mockClear();
	setBound.mockClear();
	target.mockClear();
	trigger.mockReset();
	trigger.mockImplementation(async () => {});
	const container = browser.document.createElement("div");
	browser.document.body.appendChild(container);
	root = createRoot(container as unknown as Element);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await browser.happyDOM.close();
});
afterAll(() => {
	URL.createObjectURL = createObjectUrl;
	URL.revokeObjectURL = revokeObjectUrl;
	mock.restore();
});

async function render(component = defaults) {
	const modulePath = "./CameraView.tsx?camera-renderer-test";
	const { A2UICameraView } = await import(modulePath);
	await act(async () =>
		root.render(
			<A2UICameraView
				component={component}
				componentId="camera"
				surfaceId="page"
				renderChild={() => null}
			/>,
		),
	);
}

async function start() {
	await act(async () => {
		browser.document
			.querySelector("button")
			?.dispatchEvent(new browser.MouseEvent("click", { bubbles: true }));
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
	expect(
		browser.document.querySelector('[role="alert"]')?.textContent,
	).toBeUndefined();
}

test("rendering and the editing canvas never acquire a camera", async () => {
	await render();
	expect(getMedia).not.toHaveBeenCalled();
	runtime = false;
	await render();
	expect(browser.document.querySelector("button")?.disabled).toBe(true);
	await start();
	expect(getMedia).not.toHaveBeenCalled();
});

test("interval capture waits for the preceding Event and never queues ticks", async () => {
	let finish!: () => void;
	const pending = new Promise<void>((resolve) => {
		finish = resolve;
	});
	trigger.mockImplementation(async (name) => {
		if (name === "frame") await pending;
	});
	await render();
	await start();
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 700));
	});
	expect(upload).toHaveBeenCalledTimes(1);
	expect(trigger.mock.calls.filter(([name]) => name === "frame")).toHaveLength(
		1,
	);
	expect(upload.mock.calls[0]?.[3]).toBe("remote");
	await act(async () => finish());
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 300));
	});
	expect(upload.mock.calls.length).toBeGreaterThan(1);
});

test("an interval without a frame Event does not transfer images", async () => {
	await render({ ...defaults, eventHandlers: {} });
	await start();
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 300));
	});
	expect(upload).not.toHaveBeenCalled();
});

test("Start exposes the session token for a separate capture button", async () => {
	await render({ ...defaults, intervalMs: { literalNumber: 0 } });
	await start();
	const session = setBound.mock.calls.find(
		([path]) => path === "/camera",
	)?.[1] as Record<string, unknown>;
	expect(session).toMatchObject({
		surfaceId: "page",
		componentId: "camera",
		status: "live",
	});
	expect(typeof session.sessionId).toBe("string");
});

test("leaving a route stops tracks and needs another user start", async () => {
	await render({ ...defaults, intervalMs: { literalNumber: 0 } });
	await start();
	expect(getMedia).toHaveBeenCalledTimes(1);
	pathname = "/different-screen";
	await render();
	expect(stop).toHaveBeenCalledTimes(1);
	expect(
		browser.document
			.querySelector("[data-camera-status]")
			?.getAttribute("data-camera-status"),
	).toBe("stopped");
	expect(getMedia).toHaveBeenCalledTimes(1);
});

test("browser visibility and native lock events stop tracks", async () => {
	await render({ ...defaults, intervalMs: { literalNumber: 0 } });
	await start();
	await act(async () => {
		browser.window.dispatchEvent(
			new browser.Event("flow-like:device-inactive"),
		);
	});
	expect(stop).toHaveBeenCalledTimes(1);
	await start();
	expect(getMedia).toHaveBeenCalledTimes(2);
	expect(
		browser.document
			.querySelector("[data-camera-status]")
			?.getAttribute("data-camera-status"),
	).toBe("live");
	await act(async () => {
		Object.defineProperty(browser.document, "visibilityState", {
			configurable: true,
			value: "hidden",
		});
		browser.document.dispatchEvent(new browser.Event("visibilitychange"));
	});
	expect(stop).toHaveBeenCalledTimes(2);
});

test("literal overlays apply once and expire without a render loop", async () => {
	const component = { ...defaults, intervalMs: { literalNumber: 0 } };
	await render(component);
	await start();
	const { executeCameraCommand } = await import("../camera-session");
	let frame: any;
	await act(async () => {
		frame = await executeCameraCommand(
			"camera.capture",
			{
				surfaceId: "page",
				componentId: "camera",
				sessionId: (
					trigger.mock.calls.find(([name]) => name === "ready")?.[2] as {
						sessionId: string;
					}
				).sessionId,
			},
			{ appId: "app" },
		);
	});
	await render({
		...component,
		overlays: {
			literalJson: JSON.stringify({
				...frame,
				ttlMs: 100,
				overlays: [
					{
						id: "translation",
						type: "text",
						x: 0.1,
						y: 0.2,
						text: "Hello camera",
					},
				],
			}),
		},
	});
	expect(browser.document.body.textContent).toContain("Hello camera");
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 150));
	});
	expect(browser.document.body.textContent).not.toContain("Hello camera");
});

function startedSessionId() {
	return (
		trigger.mock.calls.find(([name]) => name === "ready")?.[2] as {
			sessionId: string;
		}
	).sessionId;
}

test("audio is explicitly disclosed, stays local until requested, and stops on route exit", async () => {
	await render({
		...defaults,
		intervalMs: { literalNumber: 0 },
		audioEnabled: { literalBool: true },
	});
	expect(browser.document.querySelector("button")?.textContent).toContain(
		"Start camera and microphone",
	);
	expect(getMedia).not.toHaveBeenCalled();
	await start();
	expect(
		browser.document.querySelector('[data-camera-microphone="active"]')
			?.textContent,
	).toContain("Microphone on");
	expect(upload).not.toHaveBeenCalled();
	const { executeCameraCommand } = await import("../camera-session");
	await act(async () => {
		const clip = (await executeCameraCommand(
			"camera.captureAudio",
			{
				surfaceId: "page",
				componentId: "camera",
				sessionId: startedSessionId(),
				durationMs: 10000,
			},
			{ appId: "app", executionTarget: "remote" },
		)) as {
			audio: { type: string; size: number; url: string };
			durationMs: number;
		};
		expect(clip.audio).toMatchObject({
			type: "audio/wav",
			size: 160044,
			url: "https://storage.example/camera-frame",
		});
		expect(clip.durationMs).toBe(5000);
	});
	expect(upload.mock.calls[0]?.[3]).toBe("remote");
	pathname = "/elsewhere";
	await render();
	expect(microphone.stop).toHaveBeenCalledTimes(1);
	expect(audioClosed).toHaveBeenCalledTimes(1);
	expect(
		browser.document.querySelector('[data-camera-microphone="active"]'),
	).toBeNull();
});

test("enabling microphone on a live camera requires another explicit Start", async () => {
	await render({ ...defaults, intervalMs: { literalNumber: 0 } });
	await start();
	await render({
		...defaults,
		intervalMs: { literalNumber: 0 },
		audioEnabled: { literalBool: true },
	});
	expect(getMedia).toHaveBeenCalledTimes(1);
	expect(stop).toHaveBeenCalledTimes(1);
	expect(
		browser.document.querySelector('[data-camera-status="stopped"]'),
	).not.toBeNull();
	await start();
	expect(getMedia).toHaveBeenCalledTimes(2);
	expect(getMedia.mock.calls[1]?.[0]).toMatchObject({
		audio: { channelCount: 1 },
	});
});

test("an OS audio interruption clears the session and stops its microphone", async () => {
	await render({
		...defaults,
		intervalMs: { literalNumber: 0 },
		audioEnabled: { literalBool: true },
	});
	await start();
	await act(async () => {
		audioContext.state = "suspended";
		audioContext.dispatchEvent(new Event("statechange"));
	});
	expect(microphone.stop).toHaveBeenCalledTimes(1);
	expect(audioClosed).toHaveBeenCalledTimes(1);
	expect(
		browser.document.querySelector('[data-camera-status="stopped"]'),
	).not.toBeNull();
	expect(
		browser.document.querySelector('[data-camera-microphone="active"]'),
	).toBeNull();
});

test("audio uses the abortable temporary upload helper and preserves a local FlowPath", async () => {
	useBatchUpload = true;
	batchUpload.mockImplementation(async () => [
		{
			uploaded: {
				url: "asset://local/clip.wav",
				flowPath: { path: "clip.wav", store_ref: "temporary" },
			},
		},
	]);
	await render({
		...defaults,
		intervalMs: { literalNumber: 0 },
		audioEnabled: { literalBool: true },
	});
	await start();
	const { executeCameraCommand } = await import("../camera-session");
	await act(async () => {
		const clip = (await executeCameraCommand(
			"camera.captureAudio",
			{
				surfaceId: "page",
				componentId: "camera",
				sessionId: startedSessionId(),
				durationMs: 1000,
			},
			{ appId: "app", executionTarget: "local" },
		)) as { audio: { flowPath: unknown } };
		expect(clip.audio.flowPath).toEqual({
			path: "clip.wav",
			store_ref: "temporary",
		});
	});
	expect(upload).not.toHaveBeenCalled();
	expect(batchUpload.mock.calls[0]?.[1]?.executionTarget).toBe("local");
	expect(batchUpload.mock.calls[0]?.[1]?.signal?.aborted).toBe(false);
	await act(async () =>
		browser.window.dispatchEvent(
			new browser.Event("flow-like:device-inactive"),
		),
	);
	expect(batchUpload.mock.calls[0]?.[1]?.signal?.aborted).toBe(true);
});

test("remote audio rejects an inaccessible local URL and does not fall back after an upload failure", async () => {
	useBatchUpload = true;
	batchUpload.mockImplementation(async () => [
		{ uploaded: { url: "blob:local-only" } },
	]);
	await render({
		...defaults,
		intervalMs: { literalNumber: 0 },
		audioEnabled: { literalBool: true },
	});
	await start();
	const { executeCameraCommand } = await import("../camera-session");
	const args = {
		surfaceId: "page",
		componentId: "camera",
		sessionId: startedSessionId(),
		durationMs: 1000,
	};
	await act(async () => {
		await expect(
			executeCameraCommand("camera.captureAudio", args, {
				appId: "app",
				executionTarget: "remote",
			}),
		).rejects.toMatchObject({ code: "upload_unavailable" });
	});
	batchUpload.mockImplementation(async () => [
		{ uploaded: { url: "http://asset.localhost/local-file.wav" } },
	]);
	await act(async () => {
		await expect(
			executeCameraCommand("camera.captureAudio", args, {
				appId: "app",
				executionTarget: "remote",
			}),
		).rejects.toMatchObject({ code: "upload_unavailable" });
	});
	batchUpload.mockImplementation(async () => [{ error: "Upload cancelled" }]);
	await act(async () => {
		await expect(
			executeCameraCommand("camera.captureAudio", args, {
				appId: "app",
				executionTarget: "remote",
			}),
		).rejects.toMatchObject({ code: "upload_failed" });
	});
	expect(upload).not.toHaveBeenCalled();
});
