import {
	DeviceError,
	type DeviceAdapter,
	type DeviceContext,
} from "./device-bridge";

export interface LocationOptions {
	highAccuracy: boolean;
	maximumAgeMs: number;
	timeoutMs: number;
	requestDeadline?: number;
}

export interface LocationFix {
	geometry: { type: "Point"; coordinates: [number, number] };
	latitude: number;
	longitude: number;
	accuracy: number;
	timestamp: number;
	altitude: number | null;
	altitudeAccuracy: number | null;
	speed: number | null;
	heading: number | null;
}

export interface GeofencePermissionStatus {
	authorization:
		| "not_determined"
		| "denied"
		| "restricted"
		| "when_in_use"
		| "always";
	accuracyAuthorization: "full" | "reduced";
	backgroundSupported: boolean;
	backgroundDelivery?: "system_monitored" | "app_running";
	monitoredRegionCount: number;
	maxMonitoredRegions: number;
	error?: { code: string; message: string };
}

export const UNSUPPORTED_GEOFENCE_STATUS: GeofencePermissionStatus = {
	authorization: "not_determined",
	accuracyAuthorization: "reduced",
	backgroundSupported: false,
	monitoredRegionCount: 0,
	maxMonitoredRegions: 20,
	error: {
		code: "unsupported",
		message:
			"Native geofence monitoring is unavailable on this platform. Use an iOS or macOS app to register this Event.",
	},
};

export function normalizeLocationOptions(
	args: Record<string, unknown>,
): LocationOptions {
	const highAccuracy =
		args.highAccuracy === undefined ? false : args.highAccuracy;
	const maximumAgeMs = args.maximumAgeMs === undefined ? 0 : args.maximumAgeMs;
	const timeoutMs = args.timeoutMs === undefined ? 10_000 : args.timeoutMs;
	if (
		typeof highAccuracy !== "boolean" ||
		typeof maximumAgeMs !== "number" ||
		!Number.isInteger(maximumAgeMs) ||
		maximumAgeMs < 0 ||
		maximumAgeMs > 300_000 ||
		typeof timeoutMs !== "number" ||
		!Number.isInteger(timeoutMs) ||
		timeoutMs < 100 ||
		timeoutMs > 120_000
	)
		throw new DeviceError(
			"invalid_arguments",
			"Location requires a boolean accuracy preference, a maximum age from 0 to 300000 ms, and a timeout from 100 to 120000 ms",
		);
	if (
		args.requestDeadline !== undefined &&
		(typeof args.requestDeadline !== "number" ||
			!Number.isFinite(args.requestDeadline))
	)
		throw new DeviceError(
			"invalid_arguments",
			"Location request deadline must be an epoch timestamp in milliseconds",
		);
	return {
		highAccuracy,
		maximumAgeMs,
		timeoutMs,
		...(typeof args.requestDeadline === "number"
			? { requestDeadline: args.requestDeadline }
			: {}),
	};
}

export function normalizeLocationFix(
	raw: unknown,
	options: { maximumAgeMs: number; startedAt: number; now?: number },
): LocationFix {
	if (!raw || typeof raw !== "object")
		throw new DeviceError(
			"invalid_location",
			"The location provider returned no position",
		);
	const position = raw as Record<string, unknown>;
	const coords = (
		position.coords && typeof position.coords === "object"
			? position.coords
			: position
	) as Record<string, unknown>;
	const finite = (value: unknown): value is number =>
		typeof value === "number" && Number.isFinite(value);
	const optional = (value: unknown): value is number | null =>
		value === null || finite(value);
	const {
		latitude,
		longitude,
		accuracy,
		altitude,
		altitudeAccuracy,
		speed,
		heading,
	} = coords;
	const timestamp = position.timestamp;
	if (
		!finite(latitude) ||
		latitude < -90 ||
		latitude > 90 ||
		!finite(longitude) ||
		longitude < -180 ||
		longitude > 180 ||
		!finite(accuracy) ||
		accuracy < 0 ||
		!finite(timestamp) ||
		timestamp < 0 ||
		!optional(altitude) ||
		!optional(altitudeAccuracy) ||
		(altitudeAccuracy !== null && altitudeAccuracy < 0) ||
		!optional(speed) ||
		(speed !== null && speed < 0) ||
		!optional(heading) ||
		(heading !== null && (heading < 0 || heading >= 360))
	)
		throw new DeviceError(
			"invalid_location",
			"The location provider returned invalid coordinates or accuracy metadata",
		);
	const now = options.now ?? Date.now();
	if (timestamp > now + 5_000)
		throw new DeviceError(
			"invalid_location",
			"The location provider returned a timestamp in the future",
		);
	if (
		timestamp <
		(options.maximumAgeMs === 0
			? options.startedAt
			: now - options.maximumAgeMs)
	)
		throw new DeviceError(
			"stale_location",
			"The location provider returned a position older than requested",
		);
	if (position.geometry !== undefined) {
		const geometry = position.geometry as LocationFix["geometry"] | null;
		if (
			!geometry ||
			geometry.type !== "Point" ||
			!Array.isArray(geometry.coordinates) ||
			geometry.coordinates.length !== 2 ||
			geometry.coordinates[0] !== longitude ||
			geometry.coordinates[1] !== latitude
		)
			throw new DeviceError(
				"invalid_location",
				"The location geometry does not match its coordinates",
			);
	}
	return {
		geometry: { type: "Point", coordinates: [longitude, latitude] },
		latitude,
		longitude,
		accuracy,
		timestamp,
		altitude,
		altitudeAccuracy,
		speed,
		heading,
	};
}

function assertLocationActive(context: DeviceContext) {
	if (context.signal?.aborted)
		throw new DeviceError("cancelled", "The location request ended");
	if (context.deadline !== undefined && context.deadline <= Date.now())
		throw new DeviceError("timeout", "The location request timed out");
	if (typeof document === "undefined" || document.visibilityState !== "visible")
		throw new DeviceError(
			"foreground_required",
			"Keep this app screen open to use location",
		);
}

export function browserLocationAvailable(): boolean {
	return (
		globalThis.isSecureContext === true &&
		typeof navigator !== "undefined" &&
		!!navigator.geolocation
	);
}

/** The permission dialog and sensor share a bounded lifetime, including native suspension. */
async function withLocationLifetime<T>(
	options: LocationOptions,
	context: DeviceContext,
	run: (live: DeviceContext) => Promise<T>,
): Promise<T> {
	assertLocationActive(context);
	const controller = new AbortController();
	const deadline = Math.min(
		Date.now() + options.timeoutMs,
		context.deadline ?? Infinity,
		options.requestDeadline ?? Infinity,
	);
	if (deadline <= Date.now())
		throw new DeviceError("timeout", "The location request timed out");
	const live = { ...context, signal: controller.signal, deadline };
	let rejectInterrupted!: (error: DeviceError) => void;
	const interrupted = new Promise<never>((_, reject) => {
		rejectInterrupted = reject;
	});
	const stop = (code: string, message: string) => {
		if (controller.signal.aborted) return;
		rejectInterrupted(new DeviceError(code, message));
		controller.abort();
	};
	const cancel = () => stop("cancelled", "The location request ended");
	const inactive = () =>
		stop(
			"foreground_required",
			"Location stopped because this app screen is no longer active",
		);
	const visibility = () => {
		if (document.visibilityState !== "visible") inactive();
	};
	const timer = setTimeout(
		() => stop("timeout", "The location request timed out"),
		deadline - Date.now(),
	);
	context.signal?.addEventListener("abort", cancel, { once: true });
	document.addEventListener("visibilitychange", visibility);
	window.addEventListener("pagehide", inactive);
	window.addEventListener("flow-like:location-background", inactive);
	try {
		assertLocationActive(live);
		const result = await Promise.race([run(live), interrupted]);
		assertLocationActive(live);
		return result;
	} finally {
		clearTimeout(timer);
		context.signal?.removeEventListener("abort", cancel);
		document.removeEventListener("visibilitychange", visibility);
		window.removeEventListener("pagehide", inactive);
		window.removeEventListener("flow-like:location-background", inactive);
		controller.abort();
	}
}

/** A one-shot watch allows cancellation; getCurrentPosition has no cancellation API. */
export async function readBrowserLocation(
	options: LocationOptions,
	context: DeviceContext,
): Promise<LocationFix> {
	if (!browserLocationAvailable())
		throw new DeviceError(
			"unsupported",
			"Location requires a secure browser context with geolocation support",
		);
	return withLocationLifetime(
		options,
		context,
		(live) =>
			new Promise<LocationFix>((resolve, reject) => {
				const provider = navigator.geolocation;
				const startedAt = Date.now();
				let watchId: number | undefined;
				let settled = false;
				const clear = () => {
					if (watchId !== undefined) {
						provider.clearWatch(watchId);
						watchId = undefined;
					}
				};
				const finish = (error?: unknown, value?: LocationFix) => {
					if (settled) return;
					settled = true;
					clear();
					live.signal?.removeEventListener("abort", cancel);
					error ? reject(error) : resolve(value!);
				};
				const cancel = () =>
					finish(new DeviceError("cancelled", "The location request ended"));
				live.signal?.addEventListener("abort", cancel, { once: true });
				if (live.signal?.aborted) {
					cancel();
					return;
				}
				try {
					watchId = provider.watchPosition(
						(position) => {
							try {
								assertLocationActive(live);
								finish(
									undefined,
									normalizeLocationFix(position, {
										maximumAgeMs: options.maximumAgeMs,
										startedAt,
									}),
								);
							} catch (error) {
								// A watch can deliver its cached fix before a fresh reading.
								if (
									error instanceof DeviceError &&
									error.code === "stale_location"
								)
									return;
								finish(error);
							}
						},
						(error) => {
							// Acquisition errors can recover while this watch remains active.
							if (error.code === 2) return;
							const code =
								error.code === 1
									? "permission_denied"
									: error.code === 3
										? "timeout"
										: "position_unavailable";
							finish(
								new DeviceError(
									code,
									error.code === 1
										? "Location permission was denied"
										: error.code === 3
											? "The location request timed out"
											: "The device could not determine its location",
								),
							);
						},
						{
							enableHighAccuracy: options.highAccuracy,
							maximumAge: options.maximumAgeMs,
							timeout: Math.max(1, live.deadline! - Date.now()),
						},
					);
					if (settled) clear();
				} catch (error) {
					finish(error);
				}
			}),
	);
}

/** OS permission belongs to the host app or origin, so workflow sharing needs its own approval. */
export async function requestLocationConsent(
	context: DeviceContext,
): Promise<void> {
	assertLocationActive(context);
	const { toast } = await import("sonner");
	assertLocationActive(context);
	return new Promise<void>((resolve, reject) => {
		let settled = false;
		let id: string | number | undefined;
		const finish = (error?: DeviceError) => {
			if (settled) return;
			settled = true;
			context.signal?.removeEventListener("abort", cancel);
			if (id !== undefined) toast.dismiss(id);
			error ? reject(error) : resolve();
		};
		const cancel = () =>
			finish(
				new DeviceError(
					"permission_denied",
					"Location sharing was not approved",
				),
			);
		id = toast(`Share your location with app ${context.appId}?`, {
			description:
				context.executionTarget === "remote"
					? "Your current coordinates and accuracy will be sent to this app's server for the requesting workflow."
					: "Your current coordinates and accuracy will be available to the requesting workflow.",
			duration: Infinity,
			action: {
				label: "Share location",
				onClick: () => {
					try {
						assertLocationActive(context);
						finish();
					} catch (error) {
						finish(error as DeviceError);
					}
				},
			},
			cancel: { label: "Don't share", onClick: cancel },
			onDismiss: cancel,
		});
		context.signal?.addEventListener("abort", cancel, { once: true });
		if (context.signal?.aborted) cancel();
	});
}

export async function requestCurrentLocation(
	args: Record<string, unknown>,
	context: DeviceContext,
	adapter?: DeviceAdapter,
	consent: (context: DeviceContext) => Promise<void> = requestLocationConsent,
): Promise<LocationFix> {
	const options = normalizeLocationOptions(args);
	return withLocationLifetime(options, context, async (live) => {
		if (
			!(context.userInitiated === true && context.executionTarget === "local")
		)
			await consent(live);
		assertLocationActive(live);
		const startedAt = Date.now();
		const raw = adapter
			? await adapter(
					"location.current",
					{ ...options, requestDeadline: live.deadline },
					live,
				)
			: await readBrowserLocation(options, live);
		assertLocationActive(live);
		return normalizeLocationFix(raw, {
			maximumAgeMs: options.maximumAgeMs,
			startedAt,
		});
	});
}
