import { isTauri } from "../platform";
import { nativeKeysReady } from "./native-client";

export interface DevicePortForward {
	readonly host: "127.0.0.1";
	readonly port: number;
	readonly closed: Promise<void>;
	close(): void;
}

/** The deployed service listener a local port reaches. */
export interface DeviceServiceTarget {
	deviceId: string;
	placementId: string;
	serviceId: string;
}

export interface DevicePortForwardOptions {
	target: DeviceServiceTarget;
	port?: number;
	signal?: AbortSignal;
}

export interface ListenerLease {
	id: string;
	token: string;
	port: number;
}

/**
 * The desktop's native listener: it carries each local connection as one
 * encrypted service stream itself, so no byte crosses the webview. A port
 * whose lease is not touched within 30 s closes.
 */
export interface DevicePortBridge {
	listen(port: number, target: DeviceServiceTarget): Promise<ListenerLease>;
	touch(lease: ListenerLease): Promise<void>;
	close(lease: ListenerLease): Promise<void>;
}

/** The first port of a device may connect its tunnel before the device checks the probe. */
const LISTEN_MS = 45_000;
const CLOSE_MS = 5_000;
const TOUCH_EVERY_MS = 10_000;

async function desktopBridge(): Promise<DevicePortBridge> {
	if (!isTauri())
		throw new Error("Local service ports require Desktop Studio.");
	const { invoke } = await import("@tauri-apps/api/core");
	const args = (lease: ListenerLease) => ({ id: lease.id, token: lease.token });
	return {
		async listen(port, target) {
			await nativeKeysReady(target.deviceId);
			return invoke<ListenerLease>("device_tunnel_listen", { port, target });
		},
		touch: (lease) => invoke("device_tunnel_touch", args(lease)),
		close: (lease) => invoke("device_tunnel_close", args(lease)),
	};
}

function cancelled(): Error {
	return new DOMException("The local service port closed.", "AbortError");
}

function deadline<T>(promise: Promise<T>, milliseconds: number): Promise<T> {
	return new Promise<T>((resolve, reject) => {
		const timer = setTimeout(
			() =>
				reject(
					new Error("Desktop stopped responding to the local port bridge."),
				),
			milliseconds,
		);
		promise.then(
			(value) => {
				clearTimeout(timer);
				resolve(value);
			},
			(error) => {
				clearTimeout(timer);
				reject(error);
			},
		);
	});
}

function abortRace<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
	if (signal.aborted) {
		void promise.catch(() => {});
		return Promise.reject(signal.reason ?? cancelled());
	}
	return new Promise<T>((resolve, reject) => {
		const abort = () => {
			signal.removeEventListener("abort", abort);
			reject(signal.reason ?? cancelled());
		};
		signal.addEventListener("abort", abort, { once: true });
		promise.then(
			(value) => {
				signal.removeEventListener("abort", abort);
				resolve(value);
			},
			(error) => {
				signal.removeEventListener("abort", abort);
				reject(error);
			},
		);
	});
}

const validLease = (lease: ListenerLease) =>
	Number.isInteger(lease.port) &&
	lease.port >= 1 &&
	lease.port <= 65_535 &&
	typeof lease.id === "string" &&
	typeof lease.token === "string" &&
	lease.id.length > 0 &&
	lease.id.length <= 64 &&
	lease.token.length > 0 &&
	lease.token.length <= 64;

function checkedPort(port = 0) {
	if (!Number.isInteger(port) || port < 0 || port > 65_535)
		throw new Error(
			"Choose a local port from 1 to 65535, or 0 for an available port.",
		);
	return port;
}

/** Why a bound listener must not be published: the source was cancelled, or the lease is malformed. */
function refusalOf(lease: ListenerLease, signal?: AbortSignal) {
	if (signal?.aborted) return signal.reason ?? cancelled();
	if (!validLease(lease))
		return new Error("Desktop returned an invalid local listener.");
	return undefined;
}

async function bind(
	native: DevicePortBridge,
	options: DevicePortForwardOptions,
	port: number,
): Promise<ListenerLease> {
	const binding = native.listen(port, options.target);
	const bounded = deadline(binding, LISTEN_MS);
	const lease = await (options.signal
		? abortRace(bounded, options.signal)
		: bounded
	).catch((error: unknown) => {
		void binding.then((late) => native.close(late)).catch(() => {});
		throw error;
	});
	const refusal = refusalOf(lease, options.signal);
	if (refusal) {
		await deadline(native.close(lease), CLOSE_MS).catch(() => {});
		throw refusal;
	}
	return lease;
}

/** Touches the lease until `close`, the source signal or page hide ends the port. */
function hold(
	native: DevicePortBridge,
	lease: ListenerLease,
	signal?: AbortSignal,
): DevicePortForward {
	const page = typeof window === "undefined" ? undefined : window;
	let done = false;
	let touching = false;
	let resolveClosed!: () => void;
	const closed = new Promise<void>((resolve) => {
		resolveClosed = resolve;
	});
	const close = () => {
		if (done) return;
		done = true;
		clearInterval(heartbeat);
		signal?.removeEventListener("abort", close);
		page?.removeEventListener("pagehide", close);
		void deadline(native.close(lease), CLOSE_MS)
			.catch(() => {})
			.finally(resolveClosed);
	};
	signal?.addEventListener("abort", close, { once: true });
	page?.addEventListener("pagehide", close, { once: true });
	const heartbeat = setInterval(() => {
		if (touching || done) return;
		touching = true;
		void deadline(native.touch(lease), CLOSE_MS)
			.catch(close)
			.finally(() => {
				touching = false;
			});
	}, TOUCH_EVERY_MS);
	return { host: "127.0.0.1", port: lease.port, close, closed };
}

/** Desktop keeps plaintext on loopback; the device checks the service permission before the port opens. */
export async function startDevicePortForward(
	options: DevicePortForwardOptions,
	bridge?: DevicePortBridge,
): Promise<DevicePortForward> {
	const port = checkedPort(options.port);
	options.signal?.throwIfAborted();
	const native = bridge ?? (await desktopBridge());
	options.signal?.throwIfAborted();
	return hold(native, await bind(native, options, port), options.signal);
}
