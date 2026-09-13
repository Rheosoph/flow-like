import { type DeviceContext, DeviceError } from "./device-bridge";

export interface FrontendAudioResult {
	status: "finished";
	durationSeconds: number;
}

interface PlaybackDependencies {
	createAudio?: () => HTMLAudioElement;
	prompt?: (
		play: () => void,
		cancel: () => void,
		isLive: () => boolean,
	) => Promise<() => void>;
}

const active = new Map<string, { stop: (error: DeviceError) => void }>();
let screenEpoch = 0;

export function getFrontendAudioEpoch(): number {
	return screenEpoch;
}

/** Navigation invalidates pending commands as well as sounds already playing. */
export function stopAllFrontendAudio(): void {
	++screenEpoch;
	for (const playback of [...active.values()])
		playback.stop(
			new DeviceError(
				"screen_closed",
				"Sound playback ended because its screen closed.",
			),
		);
}

export function validateFrontendAudio(
	args: Record<string, unknown>,
	context: Pick<DeviceContext, "executionTarget">,
	origin = typeof location === "undefined" ? undefined : location.origin,
): { url: string; volume: number } {
	if (
		typeof args.url !== "string" ||
		!args.url ||
		/\p{Cc}/u.test(args.url) ||
		new TextEncoder().encode(args.url).byteLength > 8192
	)
		throw new DeviceError(
			"invalid_arguments",
			"Sound requires an audio URL of at most 8192 bytes.",
		);
	const volume = args.volume === undefined ? 1 : args.volume;
	if (
		typeof volume !== "number" ||
		!Number.isFinite(volume) ||
		volume < 0 ||
		volume > 1
	)
		throw new DeviceError(
			"invalid_arguments",
			"Sound volume must be between 0 and 1.",
		);
	let url: URL;
	try {
		url = new URL(args.url);
	} catch {
		throw new DeviceError(
			"invalid_arguments",
			"Sound requires an absolute audio URL.",
		);
	}
	if (url.username || url.password)
		throw new DeviceError(
			"invalid_arguments",
			"Sound URLs cannot include credentials.",
		);
	const local = context.executionTarget === "local";
	const hostname = url.hostname.toLowerCase().replace(/\.+$/, "");
	const assetHost = ["asset.localhost", "tauri.localhost"].includes(hostname);
	const http =
		["https:", "http:"].includes(url.protocol) && (!assetHost || local);
	const asset =
		local && url.protocol === "asset:" && url.hostname === "localhost";
	const blob =
		local &&
		url.protocol === "blob:" &&
		origin &&
		origin !== "null" &&
		url.origin === origin;
	if (!http && !asset && !blob)
		throw new DeviceError(
			"unsupported_url",
			"Use an HTTP or HTTPS audio URL. Local runs can also play asset URLs and blob URLs from this screen.",
		);
	return { url: url.href, volume };
}

async function promptToPlay(
	play: () => void,
	cancel: () => void,
	isLive: () => boolean,
): Promise<() => void> {
	const { toast } = await import("sonner");
	if (!isLive()) return () => {};
	const id = toast("Your Event has a sound ready to play", {
		duration: Number.POSITIVE_INFINITY,
		action: { label: "Play sound", onClick: play },
		cancel: { label: "Dismiss", onClick: cancel },
		onDismiss: cancel,
	});
	return () => toast.dismiss(id);
}

function playbackError(error: unknown): DeviceError {
	if (error && typeof error === "object" && "name" in error) {
		if (error.name === "NotAllowedError")
			return new DeviceError(
				"permission_required",
				"This browser did not allow sound playback.",
			);
		if (error.name === "NotSupportedError")
			return new DeviceError(
				"unsupported_format",
				"This audio format could not be played.",
			);
	}
	return new DeviceError(
		"playback_failed",
		"The sound could not be loaded or played.",
	);
}

export function playFrontendAudio(
	args: Record<string, unknown>,
	context: DeviceContext,
	dependencies: PlaybackDependencies = {},
): Promise<FrontendAudioResult> {
	return new Promise((resolve, reject) => {
		let settled = false;
		let element: HTMLAudioElement | undefined;
		let dismissPrompt: (() => void) | undefined;
		let prompting = false;
		let gestureAttempted = false;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const epoch = context.frontendAudioEpoch ?? screenEpoch;
		const deadline = Math.min(
			context.deadline ?? Date.now() + 300_000,
			Date.now() + 600_000,
		);
		const liveError = () => {
			if (context.signal?.aborted)
				return new DeviceError("cancelled", "The sound request was cancelled.");
			if (!Number.isFinite(deadline) || deadline <= Date.now())
				return new DeviceError(
					"timeout",
					"Sound playback did not finish before the request expired.",
				);
			if (epoch !== screenEpoch)
				return new DeviceError(
					"screen_closed",
					"This sound belongs to a screen that has closed.",
				);
			if (
				typeof document === "undefined" ||
				document.visibilityState !== "visible"
			)
				return new DeviceError(
					"foreground_required",
					"Keep this screen open to play sound.",
				);
			return undefined;
		};
		const stop = (error: DeviceError) => finish(error);
		const finish = (error?: DeviceError, value?: FrontendAudioResult) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			context.signal?.removeEventListener("abort", cancelled);
			if (typeof document !== "undefined")
				document.removeEventListener("visibilitychange", visibilityChanged);
			if (typeof window !== "undefined") {
				window.removeEventListener("pagehide", screenClosed);
				window.removeEventListener("flow-like:device-inactive", screenClosed);
			}
			if (active.get(context.appId)?.stop === stop)
				active.delete(context.appId);
			if (element) {
				element.removeEventListener("ended", ended);
				element.removeEventListener("error", failed);
				element.pause();
				element.removeAttribute("src");
				element.load();
			}
			dismissPrompt?.();
			error ? reject(error) : resolve(value as FrontendAudioResult);
		};
		const cancelled = () =>
			stop(new DeviceError("cancelled", "The sound request was cancelled."));
		const screenClosed = () => stopAllFrontendAudio();
		const visibilityChanged = () => {
			if (document.visibilityState !== "visible") screenClosed();
		};
		const ended = () => {
			const error = liveError();
			if (error) return stop(error);
			const duration = Number.isFinite(element?.duration)
				? element?.duration
				: element?.currentTime;
			finish(undefined, {
				status: "finished",
				durationSeconds:
					typeof duration === "number" && Number.isFinite(duration)
						? Math.max(0, duration)
						: 0,
			});
		};
		const failed = () =>
			stop(
				new DeviceError(
					element?.error?.code === 3 || element?.error?.code === 4
						? "unsupported_format"
						: "playback_failed",
					"The sound could not be loaded or decoded.",
				),
			);
		const attempt = (fromGesture = false) => {
			if (settled || (fromGesture && gestureAttempted)) return;
			const error = liveError();
			if (error) return stop(error);
			if (fromGesture) gestureAttempted = true;
			// Calling play synchronously here preserves the toast button's browser activation.
			let playing: Promise<void>;
			try {
				playing =
					element?.play() ?? Promise.reject(new Error("Missing audio element"));
			} catch (reason) {
				playing = Promise.reject(reason);
			}
			void playing.then(
				() => {
					if (settled) return;
					const error = liveError();
					if (error) stop(error);
				},
				async (reason) => {
					if (settled) return;
					if (
						!fromGesture &&
						!prompting &&
						reason &&
						typeof reason === "object" &&
						reason.name === "NotAllowedError"
					) {
						prompting = true;
						try {
							const dismiss = await (dependencies.prompt ?? promptToPlay)(
								() => attempt(true),
								cancelled,
								() => !settled && !liveError(),
							);
							if (settled) dismiss();
							else {
								dismissPrompt = dismiss;
								const error = liveError();
								if (error) stop(error);
							}
						} catch {
							stop(
								new DeviceError(
									"permission_required",
									"Allow sound playback and try this Event again.",
								),
							);
						}
					} else stop(playbackError(reason));
				},
			);
		};
		try {
			const validated = validateFrontendAudio(args, context);
			const error = liveError();
			if (error) return stop(error);
			if (!context.appId)
				throw new DeviceError(
					"invalid_arguments",
					"Sound playback requires an app invocation.",
				);
			active
				.get(context.appId)
				?.stop(
					new DeviceError(
						"interrupted",
						"A newer sound replaced this playback.",
					),
				);
			element = dependencies.createAudio?.() ?? new Audio();
			element.preload = "auto";
			element.volume = validated.volume;
			element.muted = validated.volume === 0;
			if (
				validated.volume > 0 &&
				Math.abs(element.volume - validated.volume) > 0.000001
			)
				throw new DeviceError(
					"unsupported_volume",
					"This device controls sound volume through its hardware buttons. Use volume 1 or mute with 0.",
				);
			element.loop = false;
			element.addEventListener("ended", ended);
			element.addEventListener("error", failed);
			context.signal?.addEventListener("abort", cancelled, { once: true });
			document.addEventListener("visibilitychange", visibilityChanged);
			window.addEventListener("pagehide", screenClosed);
			window.addEventListener("flow-like:device-inactive", screenClosed);
			timer = setTimeout(
				() =>
					stop(
						new DeviceError(
							"timeout",
							"Sound playback did not finish before the request expired.",
						),
					),
				Math.max(0, deadline - Date.now()),
			);
			active.set(context.appId, { stop });
			element.src = validated.url;
			attempt();
		} catch (error) {
			stop(error instanceof DeviceError ? error : playbackError(error));
		}
	});
}
