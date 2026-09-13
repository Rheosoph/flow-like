import { describe, expect, test } from "bun:test";
import {
	type DeviceReply,
	cancelDeviceCommands,
	createDeviceCommandSession,
	parseDeviceRequest,
	validateClipboard,
	withDeviceCommandBridge,
	executeDeviceCommand,
	registerDeviceAdapter,
} from "./device-bridge";
import type { IIntercomEvent } from "./schema/events/intercom-event";

const message = (overrides: Record<string, unknown> = {}) => ({
	type: "deviceCommand",
	request_id: "request",
	app_id: "app",
	command: "clipboard.write",
	args: { format: "text", text: "result" },
	timeout_ms: 10000,
	channel: {
		channel_id: "channel",
		request_id: "request",
		expires_at: Math.floor(Date.now() / 1000) + 30,
		transport: { type: "in_process" },
	},
	...overrides,
});
const event = (payload: unknown) =>
	({ event_type: "a2ui", event_id: "event", payload }) as IIntercomEvent;
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

test("only audio playback can request a timeout above two minutes", () => {
	expect(
		parseDeviceRequest(message({ command: "audio.play", timeout_ms: 900000 }))
			?.timeoutMs,
	).toBe(600000);
	expect(
		parseDeviceRequest(
			message({ command: "audio.play", timeout_ms: undefined }),
		)?.timeoutMs,
	).toBe(300000);
	expect(
		parseDeviceRequest(
			message({ command: "clipboard.write", timeout_ms: 900000 }),
		)?.timeoutMs,
	).toBe(120000);
	expect(
		parseDeviceRequest(
			message({ command: "camera.captureAudio", timeout_ms: 900000 }),
		)?.timeoutMs,
	).toBe(120000);
});

test("the live sound bridge waits for playback and cancellation releases the element", async () => {
	const keys = ["document", "window", "Audio"];
	const previous = keys.map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	let audio!: EventTarget & { src: string; paused: boolean };
	Object.defineProperties(globalThis, {
		document: {
			configurable: true,
			value: Object.assign(new EventTarget(), { visibilityState: "visible" }),
		},
		window: { configurable: true, value: new EventTarget() },
		Audio: {
			configurable: true,
			value: class extends EventTarget {
				src = "";
				volume = 1;
				muted = false;
				preload = "auto";
				loop = false;
				duration = 2;
				currentTime = 2;
				paused = true;
				constructor() {
					super();
					audio = this;
				}
				async play() {
					this.paused = false;
				}
				pause() {
					this.paused = true;
				}
				removeAttribute() {
					this.src = "";
				}
				load() {}
			},
		},
	});
	const replies: DeviceReply[] = [];
	const bridge = createDeviceCommandSession(
		{ appId: "app", executionTarget: "remote" },
		{
			reply: async (_channel, value) => {
				replies.push(value as DeviceReply);
			},
		},
	);
	try {
		bridge.filter([
			event(
				message({
					command: "audio.play",
					args: { url: "https://example.com/audio.wav" },
				}),
			),
		]);
		await flush();
		expect(replies).toHaveLength(0);
		expect(audio.paused).toBe(false);
		audio.dispatchEvent(new Event("ended"));
		await flush();
		expect(replies).toEqual([
			{ ok: true, value: { status: "finished", durationSeconds: 2 } },
		]);
		bridge.filter([
			event(
				message({
					request_id: "next",
					channel: { ...message().channel, request_id: "next" },
					command: "audio.play",
					args: { url: "https://example.com/audio.wav" },
				}),
			),
		]);
		await flush();
		bridge.close();
		await flush();
		expect(audio.paused).toBe(true);
		expect(audio.src).toBe("");
		expect(replies.at(-1)).toMatchObject({
			ok: false,
			error: { code: "cancelled" },
		});
	} finally {
		bridge.close();
		for (const [key, descriptor] of previous)
			descriptor
				? Object.defineProperty(globalThis, key, descriptor)
				: Reflect.deleteProperty(globalThis, key);
	}
});

test("camera audio commands traverse the live device bridge as scoped media references", async () => {
	const previous = Object.getOwnPropertyDescriptor(globalThis, "document");
	Object.defineProperty(globalThis, "document", {
		configurable: true,
		value: { visibilityState: "visible" },
	});
	const { CameraSession } = await import("../components/a2ui/camera-session");
	const track = Object.assign(new EventTarget(), { enabled: true, stop() {} });
	const stream = {
		getTracks: () => [track],
		getAudioTracks: () => [track],
	} as unknown as MediaStream;
	const camera = new CameraSession({
		appId: "app",
		surfaceId: "page",
		componentId: "camera",
		video: () =>
			({
				videoWidth: 640,
				videoHeight: 480,
				play: async () => {},
				pause() {},
			}) as unknown as HTMLVideoElement,
		isVisible: () => true,
		getMedia: async () => stream,
		onState() {},
		encode: async () => new Blob(["jpeg"]),
		audioRecorder: async () => ({
			snapshot: async (_duration, capturedAt) => ({
				pcm: new Float32Array(1600),
				startedAt: capturedAt - 100,
				endedAt: capturedAt,
			}),
			pause() {},
			resume() {},
			dispose() {},
		}),
		upload: async (file, context) => {
			expect(context?.executionTarget).toBe("remote");
			return {
				name: file.name,
				type: file.type,
				size: file.size,
				url: `https://storage.example/${file.name}`,
				flowPath: { path: file.name, store_ref: "temporary" },
			};
		},
	});
	const replies: DeviceReply[] = [];
	const bridge = createDeviceCommandSession(
		{ appId: "app", executionTarget: "remote" },
		{
			reply: async (_channel, value) => {
				replies.push(value as DeviceReply);
			},
		},
	);
	try {
		await camera.start({}, true);
		for (const command of ["camera.captureAudio", "camera.captureInput"]) {
			expect(
				bridge.filter([
					event(
						message({
							command,
							request_id: command,
							channel: { ...message().channel, request_id: command },
							args: {
								surfaceId: "page",
								componentId: "camera",
								sessionId: camera.state.sessionId,
								durationMs: 1000,
							},
						}),
					),
				]),
			).toEqual([]);
			await flush();
		}
		expect(replies).toHaveLength(2);
		expect(replies[0]).toMatchObject({
			ok: true,
			value: {
				sessionId: camera.state.sessionId,
				durationMs: 100,
				requestedDurationMs: 1000,
				audio: { type: "audio/wav", size: 3244 },
			},
		});
		expect(replies[1]).toMatchObject({
			ok: true,
			value: {
				frame: { sessionId: camera.state.sessionId },
				audio: {
					sessionId: camera.state.sessionId,
					audio: { type: "audio/wav" },
				},
			},
		});
		expect(JSON.stringify(replies)).not.toContain('"pcm"');
		expect(JSON.stringify(replies)).not.toContain('"bytes"');
	} finally {
		bridge.close();
		camera.dispose();
		previous
			? Object.defineProperty(globalThis, "document", previous)
			: Reflect.deleteProperty(globalThis, "document");
	}
});

describe("location device commands", () => {
	test("geofence permission requests require a deliberate local settings action", async () => {
		const previous = Object.getOwnPropertyDescriptor(globalThis, "document");
		Object.defineProperty(globalThis, "document", {
			configurable: true,
			value: { visibilityState: "visible" },
		});
		let calls = 0;
		const unregister = registerDeviceAdapter(async () => {
			calls++;
			return { authorization: "when_in_use" };
		});
		try {
			await expect(
				executeDeviceCommand(
					"location.geofencePermission",
					{ mode: "background", userInitiated: true },
					{ appId: "app", executionTarget: "remote", userInitiated: true },
				),
			).rejects.toMatchObject({ code: "permission_required" });
			await expect(
				executeDeviceCommand(
					"location.geofencePermission",
					{ mode: "foreground" },
					{ appId: "app", executionTarget: "local" },
				),
			).rejects.toMatchObject({ code: "permission_required" });
			await expect(
				executeDeviceCommand(
					"location.geofencePermission",
					{ mode: null },
					{ appId: "app", executionTarget: "local", userInitiated: true },
				),
			).rejects.toMatchObject({ code: "invalid_arguments" });
			expect(calls).toBe(0);
			expect(
				await executeDeviceCommand(
					"location.geofencePermission",
					{ mode: "status" },
					{ appId: "app", executionTarget: "local" },
				),
			).toEqual({ authorization: "when_in_use" });
			expect(
				await executeDeviceCommand(
					"location.geofencePermission",
					{ mode: "background" },
					{ appId: "app", executionTarget: "local", userInitiated: true },
				),
			).toEqual({ authorization: "when_in_use" });
			unregister();
			expect(
				await executeDeviceCommand(
					"location.geofencePermission",
					{ mode: "status" },
					{ appId: "app", executionTarget: "local" },
				),
			).toMatchObject({
				backgroundSupported: false,
				error: { code: "unsupported" },
			});
		} finally {
			unregister();
			previous
				? Object.defineProperty(globalThis, "document", previous)
				: Reflect.deleteProperty(globalThis, "document");
		}
	});
	test("keeps a remote location request scoped to its invocation and wraps the normalized result once", async () => {
		const replies: DeviceReply[] = [];
		const fix = {
			geometry: { type: "Point", coordinates: [13.4, 52.5] },
			latitude: 52.5,
			longitude: 13.4,
		};
		const session = createDeviceCommandSession(
			{ appId: "app", executionTarget: "remote" },
			{
				execute: async (command, _args, context) => {
					expect(command).toBe("location.current");
					expect(context.appId).toBe("app");
					expect(context.executionTarget).toBe("remote");
					expect(context.userInitiated).toBeUndefined();
					return fix;
				},
				reply: async (_channel, value) => {
					replies.push(value as DeviceReply);
				},
			},
		);
		expect(
			session.filter([
				event(
					message({
						command: "location.current",
						args: {
							highAccuracy: false,
							maximumAgeMs: 0,
							timeoutMs: 10000,
							userInitiated: true,
						},
					}),
				),
			]),
		).toEqual([]);
		await flush();
		expect(replies).toEqual([{ ok: true, value: fix }]);
		session.close();
	});
	test("advertises installed native location and routes a local Locate click through it", async () => {
		const keys = ["document", "window", "navigator", "isSecureContext"];
		const saved = keys.map(
			(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
		);
		Object.defineProperty(globalThis, "document", {
			configurable: true,
			value: Object.assign(new EventTarget(), { visibilityState: "visible" }),
		});
		Object.defineProperty(globalThis, "window", {
			configurable: true,
			value: new EventTarget(),
		});
		Object.defineProperty(globalThis, "navigator", {
			configurable: true,
			value: {},
		});
		Object.defineProperty(globalThis, "isSecureContext", {
			configurable: true,
			value: true,
		});
		let calls = 0;
		const unregister = registerDeviceAdapter(
			async () => {
				calls++;
				return {
					latitude: 52.5,
					longitude: 13.4,
					accuracy: 8,
					timestamp: Date.now(),
					altitude: null,
					altitudeAccuracy: null,
					speed: null,
					heading: null,
				};
			},
			{ location: "native" },
		);
		try {
			expect(
				await executeDeviceCommand("device.capabilities", {}, { appId: "app" }),
			).toMatchObject({ location: "native" });
			expect(
				await executeDeviceCommand(
					"location.current",
					{},
					{ appId: "app", executionTarget: "local", userInitiated: true },
				),
			).toMatchObject({
				geometry: { type: "Point", coordinates: [13.4, 52.5] },
			});
			expect(calls).toBe(1);
			unregister();
			expect(
				await executeDeviceCommand("device.capabilities", {}, { appId: "app" }),
			).toMatchObject({ location: false });
		} finally {
			unregister();
			for (const [key, descriptor] of saved)
				descriptor
					? Object.defineProperty(globalThis, key, descriptor)
					: Reflect.deleteProperty(globalThis, key);
		}
	});
});

describe("live frontend device bridge", () => {
	test("cancel stops a pending device operation before its execution stream closes", async () => {
		const replies: DeviceReply[] = [];
		let signal: AbortSignal | undefined;
		const session = createDeviceCommandSession(
			{ appId: "app" },
			{
				execute: async (_command, _args, context) => {
					signal = context.signal;
					return new Promise(() => {});
				},
				reply: async (_channel, value) => {
					replies.push(value as DeviceReply);
				},
			},
		);
		session.filter([
			{
				event_type: "run_initiated",
				payload: { run_id: "run-to-cancel" },
			} as IIntercomEvent,
			event(message()),
		]);
		cancelDeviceCommands("run-to-cancel");
		await flush();
		expect(signal?.aborted).toBe(true);
		expect(replies).toEqual([
			{
				ok: false,
				error: {
					code: "cancelled",
					message: "Device request expired or its run ended",
				},
			},
		]);
		session.close();
	});
	test("acknowledges the client write once and removes commands before output persistence", async () => {
		let writes = 0;
		const replies: unknown[] = [];
		const session = createDeviceCommandSession(
			{ appId: "app", executionTarget: "remote" },
			{
				execute: async (_command, args, context) => {
					writes++;
					expect(context.executionTarget).toBe("remote");
					return { copied: args.text };
				},
				reply: async (_channel, value) => {
					replies.push(value);
				},
			},
		);
		const output = {
			event_type: "completed",
			event_id: "done",
			payload: {},
		} as IIntercomEvent;
		expect(
			session.filter([event(message()), event(message()), output]),
		).toEqual([output]);
		await flush();
		expect(writes).toBe(1);
		expect(replies).toEqual([{ ok: true, value: { copied: "result" } }]);
		session.close();
	});
	test("rejects cross-app and expired requests without using a device", async () => {
		let writes = 0;
		const replies: DeviceReply[] = [];
		const session = createDeviceCommandSession(
			{ appId: "app" },
			{
				execute: async () => {
					writes++;
				},
				reply: async (_channel, value) => {
					replies.push(value as DeviceReply);
				},
			},
		);
		session.filter([
			event(message({ app_id: "other" })),
			event(
				message({
					request_id: "expired",
					channel: {
						channel_id: "channel",
						request_id: "expired",
						expires_at: 1,
						transport: { type: "in_process" },
					},
				}),
			),
		]);
		await flush();
		expect(writes).toBe(0);
		expect(replies.map((value) => !value.ok && value.error.code)).toEqual([
			"scope_mismatch",
			"expired",
		]);
		session.close();
	});
	test("closing a run aborts a pending operation and refuses late replay", async () => {
		let started = 0;
		let signal: AbortSignal | undefined;
		const replies: DeviceReply[] = [];
		const session = createDeviceCommandSession(
			{ appId: "app" },
			{
				execute: async (_command, _args, context) => {
					started++;
					signal = context.signal;
					await new Promise((resolve) =>
						context.signal?.addEventListener("abort", resolve, { once: true }),
					);
				},
				reply: async (_channel, value) => {
					replies.push(value as DeviceReply);
				},
			},
		);
		session.filter([event(message())]);
		session.close();
		session.filter([event(message({ request_id: "later" }))]);
		await flush();
		expect(signal?.aborted).toBe(true);
		expect(started).toBe(1);
		expect(replies).toEqual([
			{
				ok: false,
				error: {
					code: "cancelled",
					message: "Device request expired or its run ended",
				},
			},
		]);
	});
	test("rejects a response handle belonging to a different request", () => {
		expect(parseDeviceRequest(message({ request_id: "unrelated" }))).toBeNull();
	});
	test("validates formats, payload bounds and content expiry", () => {
		expect(() =>
			validateClipboard({
				format: "html",
				html: "<b>Result</b>",
				text: "€".repeat(400000),
			}),
		).toThrow("size");
		expect(() =>
			validateClipboard({ format: "text", text: "Hello" }),
		).not.toThrow();
		expect(() =>
			validateClipboard({ format: "text", text: "x".repeat(1024 * 1024 + 1) }),
		).toThrow("size");
		expect(() => validateClipboard({ format: "file", text: "Hello" })).toThrow(
			"format",
		);
		expect(() =>
			validateClipboard({ format: "text", text: "Hello", expiresAt: 1 }),
		).toThrow("expired");
	});
	test("a bridge does not require a UI output callback", async () => {
		await expect(
			withDeviceCommandBridge({ appId: "app" }, undefined, async (callback) => {
				callback([]);
				return 7;
			}),
		).resolves.toBe(7);
	});
});
