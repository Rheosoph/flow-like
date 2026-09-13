import { isChannelHandle, replyToChannel } from "./channel";
import { getFrontendAudioEpoch } from "./frontend-audio";
import type { IChannelHandle } from "./schema/channel";
import type { IIntercomEvent } from "./schema/events/intercom-event";

export type DeviceReply =
	| { ok: true; value?: unknown }
	| { ok: false; error: { code: string; message: string } };
export interface DeviceContext {
	appId: string;
	eventId?: string;
	executionTarget?: "local" | "remote";
	signal?: AbortSignal;
	deadline?: number;
	/** Set only by a trusted local UI click, never from workflow arguments. */
	userInitiated?: boolean;
	/** Captured by the live bridge, never accepted from workflow arguments. */
	frontendAudioEpoch?: number;
}
export type DeviceAdapter = (
	command: string,
	args: Record<string, unknown>,
	context: DeviceContext,
) => Promise<unknown>;
let nativeAdapter: DeviceAdapter | undefined;
let nativeLocationCapability: boolean | "native" | undefined;
const runCancellations = new Map<string, Set<() => void>>();

export function cancelDeviceCommands(runId: string): void {
	for (const cancel of runCancellations.get(runId) ?? []) cancel();
}

export function registerDeviceAdapter(
	adapter: DeviceAdapter,
	capabilities?: { location?: boolean | "native" },
): () => void {
	nativeAdapter = adapter;
	nativeLocationCapability = capabilities?.location;
	return () => {
		if (nativeAdapter === adapter) {
			nativeAdapter = undefined;
			nativeLocationCapability = undefined;
		}
	};
}

export class DeviceError extends Error {
	constructor(
		public code: string,
		message: string,
	) {
		super(message);
		this.name = "DeviceError";
	}
}

export function deviceFailure(error: unknown): DeviceReply {
	return {
		ok: false,
		error: {
			code:
				error &&
				typeof error === "object" &&
				"code" in error &&
				typeof error.code === "string"
					? error.code
					: error instanceof DOMException && error.name === "NotAllowedError"
						? "permission_required"
						: "device_failed",
			message: error instanceof Error ? error.message : String(error),
		},
	};
}

function assertLive(context: DeviceContext): void {
	if (context.signal?.aborted)
		throw new DeviceError("cancelled", "The invoking run has ended");
	if (context.deadline !== undefined && context.deadline <= Date.now())
		throw new DeviceError("expired", "The device request has expired");
	if (
		typeof document === "undefined" ||
		document.visibilityState === "hidden"
	) {
		throw new DeviceError(
			"foreground_required",
			"Open Flow-Like to use this device capability",
		);
	}
}

export function validateClipboard(args: Record<string, unknown>): void {
	if (!["text", "html", "image"].includes(String(args.format)))
		throw new DeviceError("unsupported_format", "Unsupported clipboard format");
	for (const key of args.format === "image"
		? ["imageBase64"]
		: args.format === "html"
			? ["html", ...(args.text === undefined ? [] : ["text"])]
			: ["text"]) {
		if (typeof args[key] !== "string")
			throw new DeviceError(
				"invalid_arguments",
				`Clipboard ${key} is required`,
			);
		const limit = key === "imageBase64" ? 24 * 1024 * 1024 : 1024 * 1024;
		if (new TextEncoder().encode(args[key] as string).byteLength > limit)
			throw new DeviceError(
				"payload_too_large",
				"Clipboard content exceeds the supported size",
			);
	}
	if (
		args.expiresAt !== undefined &&
		(typeof args.expiresAt !== "number" ||
			!Number.isFinite(args.expiresAt) ||
			args.expiresAt <= Date.now())
	) {
		throw new DeviceError("expired", "Clipboard content has expired");
	}
	if (args.localOnly !== undefined && typeof args.localOnly !== "boolean")
		throw new DeviceError(
			"invalid_arguments",
			"Keep on device must be a boolean",
		);
}

async function browserClipboard(
	args: Record<string, unknown>,
	context: DeviceContext,
): Promise<unknown> {
	validateClipboard(args);
	if (args.localOnly === true || args.expiresAt !== undefined)
		throw new DeviceError(
			"unsupported_options",
			"This browser cannot enforce clipboard expiration or local-only sharing",
		);
	if (!globalThis.isSecureContext || !navigator.clipboard)
		throw new DeviceError(
			"unsupported",
			"Clipboard access requires a secure browser context",
		);
	const write = async () => {
		assertLive(context);
		if (args.format === "text")
			await navigator.clipboard.writeText(args.text as string);
		else {
			if (typeof ClipboardItem === "undefined")
				throw new DeviceError(
					"unsupported_format",
					"This browser only supports clipboard text",
				);
			const content: Record<string, Blob> = {};
			if (args.format === "html") {
				content["text/html"] = new Blob([args.html as string], {
					type: "text/html",
				});
				if (typeof args.text === "string")
					content["text/plain"] = new Blob([args.text], { type: "text/plain" });
			} else {
				const binary = atob(args.imageBase64 as string);
				content["image/png"] = new Blob(
					[Uint8Array.from(binary, (c) => c.charCodeAt(0))],
					{ type: "image/png" },
				);
			}
			await navigator.clipboard.write([new ClipboardItem(content)]);
		}
		return { format: args.format };
	};
	try {
		return await write();
	} catch (error) {
		if (!(error instanceof DOMException && error.name === "NotAllowedError"))
			throw error;
		// A fresh click supplies browser user activation after remote work has completed.
		const { toast } = await import("sonner");
		return new Promise((resolve, reject) => {
			let settled = false;
			const finish = (error?: unknown, value?: unknown) => {
				if (settled) return;
				settled = true;
				context.signal?.removeEventListener("abort", cancel);
				toast.dismiss(id);
				error ? reject(error) : resolve(value);
			};
			const cancel = () =>
				finish(
					new DeviceError(
						"cancelled",
						"Clipboard request ended before copying",
					),
				);
			const id = toast("Your Event has content ready to copy", {
				duration: Number.POSITIVE_INFINITY,
				action: {
					label: "Copy result",
					onClick: () => {
						void write().then((value) => finish(undefined, value), finish);
					},
				},
				cancel: { label: "Dismiss", onClick: cancel },
				onDismiss: cancel,
			});
			context.signal?.addEventListener("abort", cancel, { once: true });
			if (context.signal?.aborted) cancel();
		});
	}
}

export async function executeDeviceCommand(
	command: string,
	args: Record<string, unknown>,
	context: DeviceContext,
): Promise<unknown> {
	assertLive(context);
	if (command === "audio.play") {
		const { playFrontendAudio } = await import("./frontend-audio");
		return playFrontendAudio(args, context);
	}
	if (command === "location.geofencePermission") {
		const mode = args.mode === undefined ? "status" : args.mode;
		if (
			typeof mode !== "string" ||
			!["status", "foreground", "background"].includes(mode)
		)
			throw new DeviceError(
				"invalid_arguments",
				"Unknown location permission request",
			);
		if (
			context.executionTarget !== "local" ||
			(mode !== "status" && context.userInitiated !== true)
		)
			throw new DeviceError(
				"permission_required",
				"Change location permissions from an explicit app settings action",
			);
		if (nativeAdapter) return nativeAdapter(command, { mode }, context);
		const { UNSUPPORTED_GEOFENCE_STATUS } = await import("./location");
		return UNSUPPORTED_GEOFENCE_STATUS;
	}
	if (command === "location.current") {
		const { requestCurrentLocation } = await import("./location");
		return requestCurrentLocation(args, context, nativeAdapter);
	}
	if (command.startsWith("camera.")) {
		const { executeCameraCommand } = await import(
			"../components/a2ui/camera-session"
		);
		return executeCameraCommand(command, args, context);
	}
	if (command === "device.capabilities") {
		return {
			clipboard: nativeAdapter ? "native" : !!navigator.clipboard,
			camera: !!navigator.mediaDevices?.getUserMedia,
			audio: typeof Audio !== "undefined",
			location:
				nativeLocationCapability ??
				(globalThis.isSecureContext === true && !!navigator.geolocation),
			foreground: document.visibilityState !== "hidden",
		};
	}
	if (command === "clipboard.write") {
		validateClipboard(args);
		return nativeAdapter
			? nativeAdapter(command, args, context)
			: browserClipboard(args, context);
	}
	if (nativeAdapter) return nativeAdapter(command, args, context);
	throw new DeviceError(
		"unsupported",
		`Device operation ${command} is unavailable`,
	);
}

interface DeviceRequest {
	requestId: string;
	appId: string;
	command: string;
	args: Record<string, unknown>;
	channel: IChannelHandle;
	timeoutMs: number;
}
export function parseDeviceRequest(message: unknown): DeviceRequest | null {
	if (!message || typeof message !== "object") return null;
	const value = message as Record<string, unknown>;
	if (
		value.type !== "deviceCommand" ||
		typeof value.request_id !== "string" ||
		!value.request_id ||
		typeof value.app_id !== "string" ||
		typeof value.command !== "string" ||
		!isChannelHandle(value.channel)
	)
		return null;
	if (
		value.channel.request_id !== value.request_id ||
		!value.args ||
		typeof value.args !== "object" ||
		Array.isArray(value.args)
	)
		return null;
	if (
		!Number.isFinite(value.channel.expires_at) ||
		(value.timeout_ms !== undefined &&
			(typeof value.timeout_ms !== "number" ||
				!Number.isFinite(value.timeout_ms)))
	)
		return null;
	return {
		requestId: value.request_id,
		appId: value.app_id,
		command: value.command,
		args: value.args as Record<string, unknown>,
		channel: value.channel,
		timeoutMs:
			typeof value.timeout_ms === "number"
				? Math.min(
						value.command === "audio.play" ? 600000 : 120000,
						Math.max(100, value.timeout_ms),
					)
				: value.command === "audio.play"
					? 300000
					: 30000,
	};
}

/** Only attach this to a live invocation, before output is cached or replayed. */
export function createDeviceCommandSession(
	context: DeviceContext,
	options: {
		execute?: DeviceAdapter;
		reply?: typeof replyToChannel;
		now?: () => number;
	} = {},
) {
	const frontendAudioEpoch =
		context.frontendAudioEpoch ?? getFrontendAudioEpoch();
	const controller = new AbortController();
	const seen = new Set<string>();
	const pending = new Set<AbortController>();
	const now = options.now ?? Date.now;
	const cancel = () => controller.abort();
	const runIds = new Set<string>();
	context.signal?.addEventListener("abort", cancel, { once: true });
	if (context.signal?.aborted) cancel();
	return {
		filter(events: IIntercomEvent[]): IIntercomEvent[] {
			return events.filter((event) => {
				if (
					!controller.signal.aborted &&
					event.event_type === "run_initiated" &&
					typeof event.payload?.run_id === "string"
				) {
					const id = event.payload.run_id;
					runIds.add(id);
					const cancellations =
						runCancellations.get(id) ?? new Set<() => void>();
					cancellations.add(cancel);
					runCancellations.set(id, cancellations);
				}
				if (
					event.event_type !== "a2ui" ||
					event.payload?.type !== "deviceCommand"
				)
					return true;
				const request = parseDeviceRequest(event.payload);
				if (!request || controller.signal.aborted) return false;
				const key = `${request.channel.channel_id}/${request.requestId}`;
				if (seen.has(key)) return false;
				seen.add(key);
				if (seen.size > 4096) {
					controller.abort();
					return false;
				}
				const job = new AbortController();
				pending.add(job);
				const deadline = Math.min(
					request.channel.expires_at * 1000,
					now() + request.timeoutMs,
				);
				const timeout = setTimeout(
					() =>
						job.abort(new DeviceError("timeout", "Device request timed out")),
					Math.max(0, deadline - now()),
				);
				const abort = () => job.abort();
				const interruptionError = () =>
					job.signal.reason instanceof DeviceError
						? job.signal.reason
						: new DeviceError(
								"cancelled",
								"Device request expired or its run ended",
							);
				controller.signal.addEventListener("abort", abort, { once: true });
				void (async () => {
					let response: DeviceReply;
					let onAbort: (() => void) | undefined;
					try {
						if (deadline <= now())
							throw new DeviceError("expired", "Device request has expired");
						if (request.appId !== context.appId)
							throw new DeviceError(
								"scope_mismatch",
								"The request does not belong to this app invocation",
							);
						const execute = options.execute ?? executeDeviceCommand;
						const interrupted = new Promise<never>((_resolve, reject) => {
							onAbort = () => reject(interruptionError());
							job.signal.addEventListener("abort", onAbort, { once: true });
							if (job.signal.aborted) onAbort();
						});
						const value = await Promise.race([
							execute(request.command, request.args, {
								...context,
								frontendAudioEpoch,
								signal: job.signal,
								deadline,
							}),
							interrupted,
						]);
						response = job.signal.aborted
							? deviceFailure(interruptionError())
							: { ok: true, value };
					} catch (error) {
						response = deviceFailure(error);
					}
					try {
						await (options.reply ?? replyToChannel)(request.channel, response);
					} catch (error) {
						console.warn(
							"Device acknowledgement could not be delivered",
							error,
						);
					} finally {
						clearTimeout(timeout);
						pending.delete(job);
						controller.signal.removeEventListener("abort", abort);
						if (onAbort) job.signal.removeEventListener("abort", onAbort);
					}
				})();
				return false;
			});
		},
		close() {
			controller.abort();
			for (const job of pending) job.abort();
			context.signal?.removeEventListener("abort", cancel);
			for (const id of runIds) {
				const cancellations = runCancellations.get(id);
				cancellations?.delete(cancel);
				if (!cancellations?.size) runCancellations.delete(id);
			}
		},
	};
}

export async function withDeviceCommandBridge<T>(
	context: DeviceContext,
	callback: ((events: IIntercomEvent[]) => void) | undefined,
	run: (callback: (events: IIntercomEvent[]) => void) => Promise<T>,
): Promise<T> {
	const session = createDeviceCommandSession(context);
	try {
		return await run((events) => {
			const visible = session.filter(events);
			if (visible.length) callback?.(visible);
		});
	} finally {
		session.close();
	}
}
