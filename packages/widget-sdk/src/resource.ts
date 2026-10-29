import type {
	FlwEnvelope,
	FlwMessageType,
	FlwPayloadMap,
	InitCapabilities,
} from "./protocol";

export interface WidgetResourceOptions {
	signal?: AbortSignal;
}

export interface ResourceReference {
	path: string;
	store_ref: string;
	cache_store_ref?: string | null;
}

export interface ResourceRequestPayload {
	requestId: string;
	reference: ResourceReference;
}

export interface ResourceResultPayload {
	requestId: string;
	ok: boolean;
	data?: ArrayBuffer;
	mimeType?: string;
	error?: string;
}

export type WidgetResourceErrorCode =
	| "not-granted"
	| "not-found"
	| "too-large"
	| "busy"
	| "unavailable"
	| "failed"
	| "timeout"
	| "connection-replaced"
	| "disposed";

export class WidgetResourceError extends Error {
	constructor(
		readonly code: WidgetResourceErrorCode,
		message: string,
	) {
		super(message);
		this.name = "WidgetResourceError";
	}
}

export const MAX_WIDGET_RESOURCE_BYTES = 256 * 1024 * 1024;
export const MAX_WIDGET_RESOURCES_IN_FLIGHT = 128;
export const WIDGET_RESOURCE_TIMEOUT_MS = 60_000;

const ERROR_MESSAGES: Record<WidgetResourceErrorCode, string> = {
	"not-granted": "Resource access is not granted to this widget",
	"not-found": "Resource was not found",
	"too-large": "Resource exceeds the 256 MiB transfer limit",
	busy: "Too many resource requests are pending",
	unavailable: "Resources are unavailable here",
	failed: "Resource could not be loaded",
	timeout: "Resource request timed out",
	"connection-replaced": "Widget connection replaced",
	disposed: "Widget is disposed",
};

const resourceError = (code: WidgetResourceErrorCode) =>
	new WidgetResourceError(code, ERROR_MESSAGES[code]);

function hasControlCharacters(value: string): boolean {
	for (let index = 0; index < value.length; index++) {
		const code = value.charCodeAt(index);
		if (code < 32 || code === 127) return true;
	}
	return false;
}

function normalizeReference(
	value: ResourceReference,
): ResourceReference | null {
	if (!value || typeof value !== "object" || Array.isArray(value)) return null;
	const { path, store_ref, cache_store_ref } = value;
	if (
		typeof path !== "string" ||
		!path ||
		path.length > 4096 ||
		hasControlCharacters(path) ||
		path.includes("\\") ||
		path.split("/").some((part) => part === "." || part === "..") ||
		typeof store_ref !== "string" ||
		!store_ref ||
		store_ref.length > 2048 ||
		hasControlCharacters(store_ref) ||
		(cache_store_ref != null &&
			(typeof cache_store_ref !== "string" ||
				cache_store_ref.length > 2048 ||
				hasControlCharacters(cache_store_ref)))
	)
		return null;
	return { path, store_ref, cache_store_ref: cache_store_ref ?? null };
}

function safeMimeType(value: unknown): string {
	if (
		typeof value !== "string" ||
		value.length > 255 ||
		hasControlCharacters(value)
	)
		return "application/octet-stream";
	const mime = value.split(";", 1)[0]?.trim().toLowerCase() ?? "";
	return /^[a-z0-9][a-z0-9!#$&^_.+-]{0,126}\/[a-z0-9][a-z0-9!#$&^_.+-]{0,126}$/.test(
		mime,
	)
		? mime
		: "application/octet-stream";
}

/** Fetches only resources that the host has granted to this widget instance. */
export function createWidgetResourceClient(
	post: <T extends FlwMessageType>(type: T, payload: FlwPayloadMap[T]) => void,
	capabilities: () => InitCapabilities,
) {
	let sequence = 0;
	let disposed = false;
	const pending = new Map<
		string,
		{
			resolve: (value: Blob) => void;
			reject: (reason: Error) => void;
			cleanup: () => void;
		}
	>();

	function settle(id: string, result: Blob | Error) {
		const entry = pending.get(id);
		if (!entry) return;
		pending.delete(id);
		entry.cleanup();
		if (result instanceof Error) entry.reject(result);
		else entry.resolve(result);
	}

	function cancel(id: string, error: Error) {
		if (!pending.has(id)) return;
		settle(id, error);
		try {
			post("resource:cancel", { requestId: id });
		} catch {
			// A disconnected host cannot receive cancellation; settle locally.
		}
	}

	return {
		resolveResource(
			reference: ResourceReference,
			options: WidgetResourceOptions = {},
		): Promise<Blob> {
			if (disposed) return Promise.reject(resourceError("disposed"));
			if (capabilities().resources !== true)
				return Promise.reject(resourceError("not-granted"));
			const source = normalizeReference(reference);
			if (!source)
				return Promise.reject(new TypeError("Invalid resource reference"));
			if (pending.size >= MAX_WIDGET_RESOURCES_IN_FLIGHT)
				return Promise.reject(resourceError("busy"));
			const signal = options.signal;
			if (signal?.aborted)
				return Promise.reject(
					new DOMException("Request aborted", "AbortError"),
				);
			const requestId = `resource-${++sequence}`;
			return new Promise((resolve, reject) => {
				const abort = () =>
					cancel(requestId, new DOMException("Request aborted", "AbortError"));
				const timer = setTimeout(
					() => cancel(requestId, resourceError("timeout")),
					WIDGET_RESOURCE_TIMEOUT_MS,
				);
				pending.set(requestId, {
					resolve,
					reject,
					cleanup: () => {
						clearTimeout(timer);
						signal?.removeEventListener("abort", abort);
					},
				});
				signal?.addEventListener("abort", abort, { once: true });
				try {
					post("resource:request", { requestId, reference: source });
				} catch {
					settle(requestId, resourceError("failed"));
				}
			});
		},
		handle(envelope: FlwEnvelope) {
			if (
				envelope.type === "capabilities:update" &&
				(envelope.payload as InitCapabilities | undefined)?.resources !== true
			) {
				for (const id of [...pending.keys()])
					cancel(id, resourceError("not-granted"));
				return;
			}
			if (envelope.type !== "resource:result") return;
			const value = envelope.payload as ResourceResultPayload;
			if (
				!value ||
				typeof value.requestId !== "string" ||
				!pending.has(value.requestId)
			)
				return;
			if (value.ok !== true) {
				const known =
					typeof value.error === "string" &&
					Object.hasOwn(ERROR_MESSAGES, value.error);
				const message =
					typeof value.error === "string"
						? value.error.trim().slice(0, 512)
						: "";
				settle(
					value.requestId,
					known
						? resourceError(value.error as WidgetResourceErrorCode)
						: new WidgetResourceError(
								"failed",
								message || ERROR_MESSAGES.failed,
							),
				);
				return;
			}
			try {
				if (!(value.data instanceof ArrayBuffer)) {
					settle(value.requestId, resourceError("failed"));
					return;
				}
				if (value.data.byteLength > MAX_WIDGET_RESOURCE_BYTES) {
					settle(value.requestId, resourceError("too-large"));
					return;
				}
				settle(
					value.requestId,
					new Blob([value.data], { type: safeMimeType(value.mimeType) }),
				);
			} catch {
				settle(value.requestId, resourceError("failed"));
			}
		},
		reset() {
			for (const id of [...pending.keys()])
				cancel(id, resourceError("connection-replaced"));
		},
		dispose() {
			disposed = true;
			for (const id of [...pending.keys()])
				cancel(id, resourceError("disposed"));
		},
	};
}
