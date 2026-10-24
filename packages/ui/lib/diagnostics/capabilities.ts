import {
	TIMED_OUT,
	attempt,
	attemptAsync,
	errorMessage,
	nonEmptyString,
	optionalBoolean,
	readGlobal,
	readProperty,
	withTimeout,
} from "./safe";
import {
	DIAGNOSTICS_CAPABILITY_IDS,
	type DiagnosticsCapability,
	type DiagnosticsCapabilityId,
} from "./types";

export const CAPABILITY_LABELS: Readonly<
	Record<DiagnosticsCapabilityId, string>
> = {
	webassembly: "WebAssembly",
	"wasm-simd": "WebAssembly SIMD",
	"wasm-threads": "WebAssembly threads",
	"web-workers": "Web Workers",
	"service-workers": "Service Workers",
	indexeddb: "IndexedDB",
	opfs: "Origin Private File System",
	"compression-stream": "CompressionStream",
	"broadcast-channel": "BroadcastChannel",
	"web-locks": "Web Locks",
	"scheduler-yield": "scheduler.yield()",
	clipboard: "Async Clipboard",
	notifications: "Notifications",
	websocket: "WebSocket",
	webtransport: "WebTransport",
	webcodecs: "WebCodecs",
};

export const EMPTY_WASM_MODULE = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);

/** `i32.const 0; i8x16.splat; i8x16.popcnt` returning a v128. */
export const WASM_SIMD_PROBE = new Uint8Array([
	0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0, 10, 10, 1, 8,
	0, 65, 0, 253, 15, 253, 98, 11,
]);

/** A shared memory (flags 3) and `i32.atomic.load`. */
export const WASM_THREADS_PROBE = new Uint8Array([
	0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 5, 4, 1, 3, 1, 1,
	10, 11, 1, 9, 0, 65, 0, 254, 16, 2, 0, 26, 11,
]);

const OPFS_TIMEOUT_MS = 2000;
const WEBCODECS_INTERFACES = [
	"VideoDecoder",
	"VideoEncoder",
	"AudioDecoder",
	"AudioEncoder",
	"ImageDecoder",
] as const;

function capability(
	id: DiagnosticsCapabilityId,
	available: boolean | null,
	detail?: string,
): DiagnosticsCapability {
	return detail ? { id, available, detail } : { id, available };
}

function present(name: string) {
	return typeof readGlobal(name) === "function";
}

function hasMethod(target: unknown, name: string) {
	return typeof readProperty(target, name) === "function";
}

function wasmValidates(wasm: unknown, bytes: Uint8Array<ArrayBuffer>): boolean {
	const validate = readProperty<(bytes: BufferSource) => unknown>(
		wasm,
		"validate",
	);
	if (typeof validate !== "function") return false;
	return attempt(() => validate.call(wasm, bytes) === true, false);
}

export function detectWebAssembly(wasm: unknown): boolean {
	return wasmValidates(wasm, EMPTY_WASM_MODULE);
}

export function detectWasmSimd(wasm: unknown): boolean {
	return wasmValidates(wasm, WASM_SIMD_PROBE);
}

export interface WasmThreadsEnvironment {
	readonly wasm: unknown;
	readonly sharedArrayBuffer: boolean;
	readonly crossOriginIsolated: boolean | null;
}

export function detectWasmSharedMemory(wasm: unknown): boolean {
	return wasmValidates(wasm, WASM_THREADS_PROBE);
}

export function detectWasmThreads(env: WasmThreadsEnvironment): boolean {
	return (
		env.sharedArrayBuffer &&
		env.crossOriginIsolated === true &&
		detectWasmSharedMemory(env.wasm)
	);
}

function wasmThreadsCapability(wasm: unknown) {
	const env: WasmThreadsEnvironment = {
		wasm,
		sharedArrayBuffer: present("SharedArrayBuffer"),
		crossOriginIsolated: optionalBoolean(readGlobal("crossOriginIsolated")),
	};
	return capability(
		"wasm-threads",
		detectWasmThreads(env),
		[
			`SharedArrayBuffer=${env.sharedArrayBuffer}`,
			`crossOriginIsolated=${env.crossOriginIsolated ?? "unknown"}`,
			`sharedMemory=${detectWasmSharedMemory(wasm)}`,
		].join(", "),
	);
}

function notificationsCapability() {
	const notification = readGlobal<unknown>("Notification");
	if (typeof notification !== "function")
		return capability("notifications", false);
	const permission = nonEmptyString(readProperty(notification, "permission"));
	return capability(
		"notifications",
		true,
		permission ? `permission=${permission}` : undefined,
	);
}

function clipboardCapability(navigator: unknown) {
	const insecure = readGlobal("isSecureContext") === false;
	return capability(
		"clipboard",
		hasMethod(readProperty(navigator, "clipboard"), "writeText"),
		insecure ? "isSecureContext=false" : undefined,
	);
}

function webCodecsCapability() {
	const interfaces = WEBCODECS_INTERFACES.filter(present);
	return capability(
		"webcodecs",
		present("VideoDecoder") && present("VideoEncoder"),
		interfaces.length > 0 ? interfaces.join(", ") : undefined,
	);
}

async function opfsCapability(): Promise<DiagnosticsCapability> {
	const storage = readProperty(readGlobal("navigator"), "storage");
	const getDirectory = readProperty<() => Promise<unknown>>(
		storage,
		"getDirectory",
	);
	if (typeof getDirectory !== "function") return capability("opfs", false);
	try {
		const root = await withTimeout(getDirectory.call(storage), OPFS_TIMEOUT_MS);
		return root === TIMED_OUT
			? capability("opfs", null, "getDirectory() timed out")
			: capability("opfs", true);
	} catch (error) {
		return capability("opfs", false, errorMessage(error));
	}
}

type SyncCapabilityId = Exclude<DiagnosticsCapabilityId, "opfs">;

function syncProbes(): Record<SyncCapabilityId, () => DiagnosticsCapability> {
	const navigator = readGlobal<unknown>("navigator");
	const wasm = readGlobal<unknown>("WebAssembly");
	const indexedDb = readGlobal<unknown>("indexedDB");
	return {
		webassembly: () => capability("webassembly", detectWebAssembly(wasm)),
		"wasm-simd": () => capability("wasm-simd", detectWasmSimd(wasm)),
		"wasm-threads": () => wasmThreadsCapability(wasm),
		"web-workers": () => capability("web-workers", present("Worker")),
		"service-workers": () =>
			capability(
				"service-workers",
				readProperty(navigator, "serviceWorker") !== undefined,
			),
		indexeddb: () =>
			capability(
				"indexeddb",
				typeof indexedDb === "object" && indexedDb !== null,
			),
		"compression-stream": () =>
			capability("compression-stream", present("CompressionStream")),
		"broadcast-channel": () =>
			capability("broadcast-channel", present("BroadcastChannel")),
		"web-locks": () =>
			capability(
				"web-locks",
				hasMethod(readProperty(navigator, "locks"), "request"),
			),
		"scheduler-yield": () =>
			capability(
				"scheduler-yield",
				hasMethod(readGlobal("scheduler"), "yield"),
			),
		clipboard: () => clipboardCapability(navigator),
		notifications: notificationsCapability,
		websocket: () => capability("websocket", present("WebSocket")),
		webtransport: () => capability("webtransport", present("WebTransport")),
		webcodecs: webCodecsCapability,
	};
}

export async function probeCapabilities(): Promise<DiagnosticsCapability[]> {
	const opfs = await attemptAsync(opfsCapability, capability("opfs", null));
	const probes = syncProbes();
	return DIAGNOSTICS_CAPABILITY_IDS.map((id) =>
		id === "opfs" ? opfs : attempt(probes[id], capability(id, null)),
	);
}
