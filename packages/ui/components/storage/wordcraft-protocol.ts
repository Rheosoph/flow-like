export const WORDCRAFT_CHANNEL = "flow-like-wordcraft";
export const DOCX_MIME =
	"application/vnd.openxmlformats-officedocument.wordprocessingml.document";
export const MAX_DOCX_BYTES = 50 * 1024 * 1024;

const arrayBufferByteLength = Object.getOwnPropertyDescriptor(
	ArrayBuffer.prototype,
	"byteLength",
)?.get;

function validExportBytes(
	value: unknown,
	maxBytes: number,
): value is ArrayBuffer {
	try {
		// A responsive preview receives cloned buffers in its own window realm.
		const length = arrayBufferByteLength?.call(value);
		return typeof length === "number" && length > 0 && length <= maxBytes;
	} catch {
		return false;
	}
}

export type WordcraftMessage =
	| { type: "ready" }
	| { type: "loading"; interactive: boolean; completed: number; total: number }
	| { type: "opened"; warnings?: string[]; previewOnly?: boolean }
	| { type: "save-request" }
	| { type: "dirty"; dirty: boolean }
	| { type: "error"; error: string; requestId?: string }
	| {
			type: "export-progress";
			requestId: string;
			stage: "preparing" | "rendering" | "finalizing";
			progress: number | null;
	  }
	| {
			type: "exported";
			requestId: string;
			bytes: ArrayBuffer;
			revision: number;
			warnings?: string[];
	  };

export function readWordcraftMessage(
	event: MessageEvent,
	frame: Window | null,
	origin: string,
	session: string,
	channel = WORDCRAFT_CHANNEL,
	maxBytes = MAX_DOCX_BYTES,
): WordcraftMessage | null {
	if (!frame || event.source !== frame || event.origin !== origin) return null;
	const data = event.data;
	if (
		!data ||
		typeof data !== "object" ||
		data.channel !== channel ||
		data.session !== session
	)
		return null;
	const warnings = Array.isArray(data.warnings)
		? data.warnings
				.filter((value: unknown): value is string => typeof value === "string")
				.slice(0, 8)
				.map((value: string) => value.slice(0, 500))
		: undefined;
	switch (data.type) {
		case "ready":
		case "save-request":
			return { type: data.type };
		case "opened":
			return {
				type: "opened",
				...(warnings?.length ? { warnings } : {}),
				...(data.previewOnly === true ? { previewOnly: true } : {}),
			};
		case "loading":
			if (
				typeof data.interactive !== "boolean" ||
				!Number.isSafeInteger(data.completed) ||
				!Number.isSafeInteger(data.total) ||
				data.completed < 0 ||
				data.total < data.completed
			)
				return null;
			return {
				type: "loading",
				interactive: data.interactive,
				completed: data.completed,
				total: data.total,
			};
		case "dirty":
			return typeof data.dirty === "boolean"
				? { type: "dirty", dirty: data.dirty }
				: null;
		case "error":
			return typeof data.error === "string"
				? {
						type: "error",
						error: data.error.slice(0, 1000),
						requestId:
							typeof data.requestId === "string" ? data.requestId : undefined,
					}
				: null;
		case "export-progress":
			if (
				typeof data.requestId !== "string" ||
				!["preparing", "rendering", "finalizing"].includes(data.stage) ||
				(data.progress !== null &&
					(typeof data.progress !== "number" ||
						!Number.isFinite(data.progress) ||
						data.progress < 0 ||
						data.progress > 100))
			)
				return null;
			return {
				type: "export-progress",
				requestId: data.requestId,
				stage: data.stage,
				progress: data.progress,
			};
		case "exported":
			if (
				typeof data.requestId !== "string" ||
				!validExportBytes(data.bytes, maxBytes) ||
				!Number.isSafeInteger(data.revision) ||
				data.revision < 0
			)
				return null;
			return {
				type: "exported",
				requestId: data.requestId,
				bytes: data.bytes,
				revision: data.revision,
				...(warnings?.length ? { warnings } : {}),
			};
		default:
			return null;
	}
}
