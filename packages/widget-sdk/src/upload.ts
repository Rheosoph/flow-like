import type { JsonSchema } from "./contract";
import type {
	FlwEnvelope,
	FlwMessageType,
	FlwPayloadMap,
	InitCapabilities,
} from "./protocol";

// The widget only ever holds this handle: the host swaps it for the stored
// file when an event carries it, so a widget can never name a storage path.
// The bundler turns `@upload` into `{"type":"object","x-flow-like-type":"file"}`.
/**
 * A file the viewer uploaded through this widget.
 *
 * @upload
 */
export interface WidgetFile {
	$flwFile: string;
	name: string;
	size: number;
	mimeType: string;
}

export interface WidgetUploadOptions {
	/** Defaults to the File's name. */
	name?: string;
	signal?: AbortSignal;
}

export interface FileUploadPayload {
	requestId: string;
	file: Blob;
	name: string;
	mimeType: string;
}

export type WidgetUploadErrorCode =
	| "not-granted"
	| "too-large"
	| "busy"
	| "sign-in-required"
	| "unavailable"
	| "failed"
	| "timeout"
	| "disposed";

export interface FileResultPayload {
	requestId: string;
	ok: boolean;
	file?: WidgetFile;
	error?: WidgetUploadErrorCode;
}

export class WidgetUploadError extends Error {
	readonly code: WidgetUploadErrorCode;

	constructor(code: WidgetUploadErrorCode, message: string) {
		super(message);
		this.name = "WidgetUploadError";
		this.code = code;
	}
}

export const MAX_WIDGET_UPLOAD_BYTES = 25 * 1024 * 1024;
export const MAX_WIDGET_UPLOADS_IN_FLIGHT = 4;
export const WIDGET_FILE_HANDLE_PATTERN = "^flwf_[A-Za-z0-9_-]{16,64}$";
const UPLOAD_TIMEOUT_MS = 10 * 60_000;
const HANDLE = new RegExp(WIDGET_FILE_HANDLE_PATTERN);

const ERROR_MESSAGES: Record<WidgetUploadErrorCode, string> = {
	"not-granted": "Uploads are not granted to this widget",
	"too-large": "Files must be 25 MB or smaller",
	busy: "Too many uploads in progress",
	"sign-in-required": "Sign in to upload files",
	unavailable: "Uploads are unavailable here",
	failed: "Upload failed",
	timeout: "Upload timed out",
	disposed: "Widget is disposed",
};

const uploadError = (code: WidgetUploadErrorCode) =>
	new WidgetUploadError(code, ERROR_MESSAGES[code]);

const isErrorCode = (value: unknown): value is WidgetUploadErrorCode =>
	typeof value === "string" && Object.hasOwn(ERROR_MESSAGES, value);

/** The wire form of a `WidgetFile`, checked wherever a contract marks one. */
export const WIDGET_FILE_SCHEMA: JsonSchema = {
	type: "object",
	required: ["$flwFile", "name", "size", "mimeType"],
	properties: {
		$flwFile: { type: "string", pattern: WIDGET_FILE_HANDLE_PATTERN },
		name: { type: "string", minLength: 1, maxLength: 255 },
		size: { type: "integer", minimum: 0 },
		mimeType: { type: "string", maxLength: 255 },
	},
	additionalProperties: false,
};

export function isWidgetFile(value: unknown): value is WidgetFile {
	if (typeof value !== "object" || value === null) return false;
	const file = value as Record<string, unknown>;
	return (
		Object.keys(file).length === 4 &&
		typeof file.$flwFile === "string" &&
		HANDLE.test(file.$flwFile) &&
		typeof file.name === "string" &&
		file.name.length > 0 &&
		file.name.length <= 255 &&
		Number.isInteger(file.size) &&
		(file.size as number) >= 0 &&
		typeof file.mimeType === "string" &&
		file.mimeType.length <= 255
	);
}

/** The handle schema for a contract schema carrying the `file` marker. */
export function markedUploadSchema(schema: JsonSchema): JsonSchema | undefined {
	return schema["x-flow-like-type"] === "file" ? WIDGET_FILE_SCHEMA : undefined;
}

export function createWidgetUploadClient(
	post: <T extends FlwMessageType>(type: T, payload: FlwPayloadMap[T]) => void,
	capabilities: () => InitCapabilities,
) {
	let sequence = 0;
	let disposed = false;
	const pending = new Map<
		string,
		{
			resolve: (value: WidgetFile) => void;
			reject: (reason: Error) => void;
			cleanup: () => void;
		}
	>();
	function settle(id: string, value: Error | WidgetFile) {
		const entry = pending.get(id);
		if (!entry) return;
		pending.delete(id);
		entry.cleanup();
		if (value instanceof Error) entry.reject(value);
		else entry.resolve(value);
	}
	return {
		uploadFile(
			file: Blob,
			options: WidgetUploadOptions = {},
		): Promise<WidgetFile> {
			if (disposed) return Promise.reject(uploadError("disposed"));
			if (!capabilities().uploads)
				return Promise.reject(uploadError("not-granted"));
			if (!(file instanceof Blob))
				return Promise.reject(
					new TypeError("uploadFile expects a File or Blob"),
				);
			if (file.size > MAX_WIDGET_UPLOAD_BYTES)
				return Promise.reject(uploadError("too-large"));
			if (pending.size >= MAX_WIDGET_UPLOADS_IN_FLIGHT)
				return Promise.reject(uploadError("busy"));
			const signal = options.signal;
			if (signal?.aborted)
				return Promise.reject(
					new DOMException("Request aborted", "AbortError"),
				);
			const fileName =
				typeof File !== "undefined" && file instanceof File ? file.name : "";
			const name = ((options.name ?? fileName) || "file").slice(0, 255);
			const requestId = `upload-${++sequence}`;
			return new Promise((resolve, reject) => {
				const cancel = () => {
					post("file:cancel", { requestId });
					settle(requestId, new DOMException("Request aborted", "AbortError"));
				};
				const timer = setTimeout(() => {
					post("file:cancel", { requestId });
					settle(requestId, uploadError("timeout"));
				}, UPLOAD_TIMEOUT_MS);
				const cleanup = () => {
					clearTimeout(timer);
					signal?.removeEventListener("abort", cancel);
				};
				pending.set(requestId, { resolve, reject, cleanup });
				signal?.addEventListener("abort", cancel, { once: true });
				post("file:upload", {
					requestId,
					file,
					name,
					mimeType: file.type || "application/octet-stream",
				});
			});
		},
		handle(envelope: FlwEnvelope) {
			if (envelope.type !== "file:result") return;
			const value = envelope.payload as FileResultPayload;
			if (!value || typeof value.requestId !== "string") return;
			if (value.ok === true && isWidgetFile(value.file)) {
				settle(value.requestId, value.file);
				return;
			}
			settle(
				value.requestId,
				uploadError(
					value.ok !== true && isErrorCode(value.error)
						? value.error
						: "failed",
				),
			);
		},
		reset() {
			for (const id of [...pending.keys()])
				settle(id, new Error("Widget connection replaced"));
		},
		dispose() {
			disposed = true;
			for (const id of [...pending.keys()]) {
				post("file:cancel", { requestId: id });
				settle(id, uploadError("disposed"));
			}
		},
	};
}
