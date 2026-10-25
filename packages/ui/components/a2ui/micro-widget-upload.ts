import {
	type FileResultPayload,
	type FlwEnvelope,
	MAX_WIDGET_UPLOADS_IN_FLIGHT,
	MAX_WIDGET_UPLOAD_BYTES,
	type WidgetFile,
	type WidgetUploadErrorCode,
	isWidgetFile,
} from "@flow-like/widget-sdk";
import {
	type ITemporaryUploadResult,
	uploadTemporaryFilesLocally,
} from "../../lib/temporary-upload-batch";
import type {
	IHelperState,
	ITemporaryFlowPath,
	ITemporaryUploadExecutionTarget,
	ITemporaryUploadedFile,
} from "../../state/backend-state/helper-state";
import { TokenBucket } from "./micro-widget-host";

export const MAX_WIDGET_UPLOADS_PER_MINUTE = 20;

const REQUEST_ID = /^[a-zA-Z0-9_-]{1,128}$/;
const MIME_TYPE = /^[\w!#$&^.+-]{1,127}\/[\w!#$&^.+-]{1,127}$/;
const HANDLE_ALPHABET =
	"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const FALLBACK_MIME_TYPE = "application/octet-stream";

/** What a workflow receives in place of a handle: the value of a FileInput upload. */
export interface ResolvedWidgetFile {
	name: string;
	size: number;
	type: string;
	url: string;
	flowPath?: ITemporaryFlowPath;
}

export interface WidgetFileRecord {
	handle: WidgetFile;
	file: ResolvedWidgetFile;
}

export type WidgetFileRegistry = ReadonlyMap<string, WidgetFileRecord>;

export type WidgetFileResolution =
	| { ok: true; payload: unknown }
	| { ok: false };

/** The only failures an upload may report to the widget; backend text never crosses. */
export type WidgetUploadFailure = Extract<
	WidgetUploadErrorCode,
	"sign-in-required" | "unavailable" | "failed"
>;

const UPLOAD_FAILURES: ReadonlySet<string> = new Set<WidgetUploadFailure>([
	"sign-in-required",
	"unavailable",
	"failed",
]);

export type WidgetUploadOutcome =
	| { ok: true; url: string; flowPath?: ITemporaryFlowPath }
	| { ok: false; error: WidgetUploadFailure };

export type WidgetUploadFunction = (
	file: File,
	signal: AbortSignal,
) => Promise<WidgetUploadOutcome>;

export function mintWidgetFileHandle(): string {
	const bytes = crypto.getRandomValues(new Uint8Array(24));
	let handle = "flwf_";
	for (const byte of bytes) handle += HANDLE_ALPHABET[byte & 63];
	return handle;
}

const UNRESOLVED = Symbol("unresolved");

const isWalkable = (value: unknown): value is object =>
	typeof value === "object" &&
	value !== null &&
	(Array.isArray(value) ||
		Object.prototype.toString.call(value) === "[object Object]");

function resolveHandle(value: object, registry: WidgetFileRegistry) {
	if (!isWidgetFile(value)) return UNRESOLVED;
	const record = registry.get(value.$flwFile);
	if (
		!record ||
		record.handle.name !== value.name ||
		record.handle.size !== value.size ||
		record.handle.mimeType !== value.mimeType
	)
		return UNRESOLVED;
	const { flowPath, ...file } = record.file;
	return flowPath ? { ...file, flowPath: { ...flowPath } } : file;
}

const substitute = (
	value: unknown,
	registry: WidgetFileRegistry,
	ancestors: Set<object>,
): unknown => {
	if (!isWalkable(value)) return value;
	if (ancestors.has(value)) return UNRESOLVED;
	if (!Array.isArray(value) && Object.hasOwn(value, "$flwFile"))
		return resolveHandle(value, registry);
	ancestors.add(value);
	const changed = substituteChildren(value, registry, ancestors);
	ancestors.delete(value);
	if (changed === UNRESOLVED) return UNRESOLVED;
	return changed.size === 0 ? value : rebuild(value, changed);
};

function substituteChildren(
	value: object,
	registry: WidgetFileRegistry,
	ancestors: Set<object>,
) {
	const changed = new Map<string, unknown>();
	for (const [key, item] of Object.entries(value)) {
		const next = substitute(item, registry, ancestors);
		if (next === UNRESOLVED) return UNRESOLVED;
		if (next !== item) changed.set(key, next);
	}
	return changed;
}

function rebuild(value: object, changed: ReadonlyMap<string, unknown>) {
	const pick = (key: string, item: unknown) =>
		changed.has(key) ? changed.get(key) : item;
	if (Array.isArray(value))
		return value.map((item, index) => pick(String(index), item));
	return Object.fromEntries(
		Object.entries(value).map(([key, item]) => [key, pick(key, item)]),
	);
}

/**
 * Swaps every file handle in an event payload for the file this host uploaded.
 * A `$flwFile` object that is not exactly a handle minted for this widget
 * instance fails the whole payload, so a widget can never name a stored file.
 */
export function resolveWidgetFileHandles(
	payload: unknown,
	registry: WidgetFileRegistry,
): WidgetFileResolution {
	const resolved = substitute(payload, registry, new Set());
	return resolved === UNRESOLVED
		? { ok: false }
		: { ok: true, payload: resolved };
}

function uploadName(value: unknown) {
	return (typeof value === "string" ? value.slice(0, 255) : "") || "file";
}

function uploadMimeType(value: unknown, fallback: string) {
	if (typeof value === "string" && MIME_TYPE.test(value)) return value;
	return MIME_TYPE.test(fallback) ? fallback : FALLBACK_MIME_TYPE;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readFileRequest(envelope: FlwEnvelope) {
	if (envelope.type !== "file:upload" && envelope.type !== "file:cancel")
		return null;
	const payload: unknown = envelope.payload;
	if (
		!isRecord(payload) ||
		typeof payload.requestId !== "string" ||
		!REQUEST_ID.test(payload.requestId)
	)
		return null;
	return { payload, requestId: payload.requestId };
}

/**
 * Host side of `file:upload`: uploads files a widget hands over and answers
 * with a handle. Handles live only in this controller, one per document, and
 * resolve only through {@link resolveWidgetFileHandles}.
 */
export function createMicroWidgetUpload(options: {
	enabled: () => boolean;
	upload: WidgetUploadFunction;
	result: (payload: FileResultPayload) => void;
	now?: () => number;
}) {
	let disposed = false;
	const now = options.now ?? Date.now;
	const rate = new TokenBucket(
		MAX_WIDGET_UPLOADS_PER_MINUTE,
		MAX_WIDGET_UPLOADS_PER_MINUTE / 60,
	);
	const inFlight = new Map<string, AbortController>();
	const files = new Map<string, WidgetFileRecord>();
	const fail = (requestId: string, error: WidgetUploadErrorCode) =>
		options.result({ requestId, ok: false, error });

	function stop(error?: WidgetUploadErrorCode) {
		const pending = [...inFlight];
		inFlight.clear();
		files.clear();
		for (const [requestId, controller] of pending) {
			controller.abort();
			if (error) fail(requestId, error);
		}
	}

	async function run(
		requestId: string,
		controller: AbortController,
		file: File,
		handle: Omit<WidgetFile, "$flwFile">,
	) {
		let outcome: WidgetUploadOutcome;
		try {
			outcome = await options.upload(file, controller.signal);
		} catch {
			outcome = { ok: false, error: "failed" };
		}
		if (disposed || inFlight.get(requestId) !== controller) return;
		inFlight.delete(requestId);
		if (!outcome.ok) {
			fail(
				requestId,
				UPLOAD_FAILURES.has(outcome.error) ? outcome.error : "failed",
			);
			return;
		}
		const minted: WidgetFile = { $flwFile: mintWidgetFileHandle(), ...handle };
		files.set(minted.$flwFile, {
			handle: minted,
			file: {
				name: handle.name,
				size: handle.size,
				type: handle.mimeType,
				url: outcome.url,
				...(outcome.flowPath && { flowPath: outcome.flowPath }),
			},
		});
		options.result({ requestId, ok: true, file: { ...minted } });
	}

	function start(request: Record<string, unknown>, requestId: string) {
		if (inFlight.has(requestId)) return;
		if (!options.enabled()) return fail(requestId, "not-granted");
		const blob = request.file;
		if (!(blob instanceof Blob)) return fail(requestId, "failed");
		if (blob.size > MAX_WIDGET_UPLOAD_BYTES)
			return fail(requestId, "too-large");
		if (inFlight.size >= MAX_WIDGET_UPLOADS_IN_FLIGHT || !rate.tryTake(now()))
			return fail(requestId, "busy");
		const name = uploadName(request.name);
		const mimeType = uploadMimeType(request.mimeType, blob.type);
		const file = new File([blob], name, {
			type: mimeType,
			...(blob instanceof File && { lastModified: blob.lastModified }),
		});
		const controller = new AbortController();
		inFlight.set(requestId, controller);
		void run(requestId, controller, file, {
			name,
			size: blob.size,
			mimeType,
		});
	}

	return {
		handle(envelope: FlwEnvelope) {
			if (disposed) return;
			const request = readFileRequest(envelope);
			if (!request) return;
			if (envelope.type === "file:upload") {
				start(request.payload, request.requestId);
				return;
			}
			inFlight.get(request.requestId)?.abort();
			inFlight.delete(request.requestId);
		},
		resolve(payload: unknown): WidgetFileResolution {
			return resolveWidgetFileHandles(payload, files);
		},
		/** Cancels running uploads with `error` and forgets every handle. */
		revoke(error: WidgetUploadErrorCode = "not-granted") {
			if (!disposed) stop(error);
		},
		dispose() {
			disposed = true;
			stop();
		},
	};
}

export type MicroWidgetUpload = ReturnType<typeof createMicroWidgetUpload>;

export interface WidgetUploadTarget {
	helper: Pick<
		IHelperState,
		"filesToTemporaryFiles" | "fileToTemporaryFile" | "fileToUrl"
	>;
	appId?: string;
	signedIn: boolean;
	executionTarget?: () => Promise<ITemporaryUploadExecutionTarget>;
}

/** Workflows fetch http(s) files; desktop runs also decode their own `asset:` files. */
function isWorkflowReadableUrl(url: string): boolean {
	try {
		return ["https:", "http:", "asset:"].includes(new URL(url).protocol);
	} catch {
		return false;
	}
}

function readFlowPath(value: unknown): ITemporaryFlowPath | undefined {
	if (
		!isRecord(value) ||
		typeof value.path !== "string" ||
		!value.path ||
		typeof value.store_ref !== "string" ||
		!value.store_ref
	)
		return undefined;
	return {
		path: value.path,
		store_ref: value.store_ref,
		...(typeof value.cache_store_ref === "string" &&
			value.cache_store_ref && { cache_store_ref: value.cache_store_ref }),
	};
}

/** FileInput's upload: the bulk path, or the single-file helper through the local batcher. */
async function uploadTemporaryFile(
	file: File,
	helper: WidgetUploadTarget["helper"],
	appId: string,
	executionTarget: ITemporaryUploadExecutionTarget | undefined,
	signal: AbortSignal,
) {
	if (helper.filesToTemporaryFiles)
		return helper.filesToTemporaryFiles([file], {
			appId,
			executionTarget,
			signal,
		});
	return uploadTemporaryFilesLocally(
		[file],
		async (item) =>
			(await helper.fileToTemporaryFile?.(
				item,
				false,
				appId,
				executionTarget,
			)) ?? {
				url: await helper.fileToUrl(item, false, appId, executionTarget),
			},
		{ signal },
	);
}

/** A stored file a workflow can read: a FlowPath, or a URL it can fetch. */
function storedFileOutcome(
	uploaded: ITemporaryUploadedFile,
	bulk: boolean,
): WidgetUploadOutcome {
	const url = typeof uploaded.url === "string" ? uploaded.url : "";
	const readable = isWorkflowReadableUrl(url);
	const flowPath = readFlowPath(uploaded.flowPath);
	if (flowPath) return { ok: true, url: readable ? url : "", flowPath };
	if (readable) return { ok: true, url };
	if (url.startsWith("blob:")) URL.revokeObjectURL(url);
	return {
		ok: false,
		error:
			bulk && /^(blob|data):/i.test(url) ? "sign-in-required" : "unavailable",
	};
}

/**
 * Uploads into the viewer's temporary storage for this app exactly as a
 * FileInput does. Signed-out viewers never reach storage: the web app hands
 * them a `blob:` URL and the desktop refuses remote uploads without a session.
 */
export async function uploadWidgetFile(
	file: File,
	target: WidgetUploadTarget,
	signal: AbortSignal,
): Promise<WidgetUploadOutcome> {
	const { helper, appId } = target;
	if (!appId || (!helper.filesToTemporaryFiles && !helper.fileToTemporaryFile))
		return { ok: false, error: "unavailable" };
	// Hosts without the bulk path (hosted pages) keep no storage for any viewer.
	const bulk = Boolean(helper.filesToTemporaryFiles);
	const failure: WidgetUploadOutcome = {
		ok: false,
		error: !bulk
			? "unavailable"
			: target.signedIn
				? "failed"
				: "sign-in-required",
	};
	let result: ITemporaryUploadResult | undefined;
	try {
		const executionTarget = await target.executionTarget?.();
		signal.throwIfAborted();
		[result] = await uploadTemporaryFile(
			file,
			helper,
			appId,
			executionTarget,
			signal,
		);
	} catch {
		return failure;
	}
	return result?.uploaded ? storedFileOutcome(result.uploaded, bulk) : failure;
}
