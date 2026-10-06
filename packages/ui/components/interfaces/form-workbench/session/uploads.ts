/*
 * The form's file uploads (spec F, M3; PLAN §3.4). A picked file uploads on its own call, with
 * its own AbortController, when the reducer asks (`upload` effect, at most two ahead):
 * - a FlowPath field: `filesToTemporaryFiles([file], { appId, eventId, executionTarget, signal,
 *   onProgress })`; the result must carry a FlowPath, it never falls back to a URL;
 * - a legacy PathBuf / Byte field: the same call without app, event and target, which gives the
 *   remote URL today's `fileToUrl(file, false)` gives.
 * Hosts that send files inline (hosted links, the device page) never upload: their slots are
 * encoded with `fileToUrl` when their run is sent (`encodeInlineValues`).
 */
import type { BulkUploadProgressCallback } from "../../../../lib/bulk-upload";
import type { ITemporaryUploadResult } from "../../../../lib/temporary-upload-batch";
import type {
	IHelperState,
	ITemporaryFlowPath,
} from "../../../../state/backend-state/helper-state";
import type {
	CopyValue,
	ExecutionTarget,
	FileMode,
	FileSlot,
	FlowPath,
	SessionInput,
	WorkbenchField,
} from "../contracts";
import { isFileField, isFileSlot } from "../model/values";

/** Spec F: a FlowPath upload that came back without one. */
export const NO_FLOW_PATH_ERROR =
	"This file could not be prepared for the flow.";
export const NO_UPLOAD_ERROR = "This page cannot upload files.";
export const NO_URL_ERROR = "The upload returned no address for this file.";
export const NO_FILE_ERROR =
	"The picked file is no longer available. Pick it again.";

export type UploadHelper = Pick<IHelperState, "filesToTemporaryFiles">;
export type InlineEncoder = Pick<IHelperState, "fileToUrl">;
type UploadOptions = NonNullable<
	Parameters<NonNullable<IHelperState["filesToTemporaryFiles"]>>[1]
>;

export interface UploadRequest {
	readonly slotId: string;
	readonly file: File;
	readonly mode: FileMode;
	readonly appId: string;
	readonly eventId: string;
	/** Where the run will execute; only FlowPath uploads wait for it. */
	readonly target: () => Promise<ExecutionTarget>;
	readonly helper: UploadHelper;
	/** `uploadProgress`, `uploadSent` or `uploadFailed`; nothing after `abort()`. */
	readonly report: (input: SessionInput) => void;
}

export interface Upload {
	readonly slotId: string;
	abort(): void;
	/** Settles once the upload reported its end, failed quietly after an abort, or was aborted. */
	readonly done: Promise<void>;
}

const messageOf = (error: unknown, fallback: string) => {
	if (error instanceof Error && error.message.trim()) return error.message;
	if (typeof error === "string" && error.trim()) return error;
	return fallback;
};

/** ms epoch of an ISO time, or null. */
export function expiresAtOf(text: string | null | undefined): number | null {
	if (typeof text !== "string" || !text) return null;
	const at = Date.parse(text);
	return Number.isFinite(at) ? at : null;
}

const nonEmpty = (value: unknown): value is string =>
	typeof value === "string" && value.length > 0;

/** A usable FlowPath from an upload result: a path and a store reference. */
export function flowPathOf(
	value: ITemporaryFlowPath | null | undefined,
): FlowPath | null {
	if (!value || !nonEmpty(value.path) || !nonEmpty(value.store_ref))
		return null;
	return {
		path: value.path,
		store_ref: value.store_ref,
		cache_store_ref: nonEmpty(value.cache_store_ref)
			? value.cache_store_ref
			: null,
	};
}

const failed = (slotId: string, error: string): SessionInput => ({
	type: "uploadFailed",
	slotId,
	error,
});

/**
 * What one upload result means for the session. A FlowPath slot keeps no expiry: it can be sent
 * again while the page is open (PLAN question 17); a URL stops at its `downloadExpiresAt`.
 */
export function uploadInputOf(
	slotId: string,
	mode: FileMode,
	result: ITemporaryUploadResult | undefined,
): SessionInput {
	const uploaded = result?.uploaded;
	if (!uploaded)
		return failed(slotId, messageOf(result?.error, "The upload failed."));
	if (mode === "flowpath") {
		const flowPath = flowPathOf(uploaded.flowPath);
		if (!flowPath) return failed(slotId, NO_FLOW_PATH_ERROR);
		const url = nonEmpty(uploaded.url) ? uploaded.url : null;
		return {
			type: "uploadSent",
			slotId,
			ref: { kind: "flowpath", flowPath, url },
			expiresAt: null,
		};
	}
	if (!nonEmpty(uploaded.url)) return failed(slotId, NO_URL_ERROR);
	return {
		type: "uploadSent",
		slotId,
		ref: { kind: "url", url: uploaded.url },
		expiresAt: expiresAtOf(uploaded.downloadExpiresAt),
	};
}

/** Progress as the slot shows it (0–1), reported only when it moved by a percent or reached the end. */
function progressReporter(
	slotId: string,
	report: (input: SessionInput) => void,
): BulkUploadProgressCallback {
	let last = -1;
	return (percent) => {
		const progress = Math.min(1, Math.max(0, percent / 100));
		if (!Number.isFinite(progress)) return;
		if (progress < 1 && progress - last < 0.01) return;
		if (progress === last) return;
		last = progress;
		report({ type: "uploadProgress", slotId, progress });
	};
}

/** The helper call; a synchronous throw, like a missing method, becomes a rejection. */
function callHelper(
	request: UploadRequest,
	target: ExecutionTarget | null,
	signal: AbortSignal,
	onProgress: BulkUploadProgressCallback,
): Promise<ITemporaryUploadResult[]> {
	const options: UploadOptions =
		request.mode === "flowpath"
			? {
					appId: request.appId,
					eventId: request.eventId,
					executionTarget: target ?? undefined,
					signal,
					onProgress,
				}
			: { signal, onProgress };
	return new Promise((resolve) => {
		const upload = request.helper.filesToTemporaryFiles;
		if (!upload) throw new Error(NO_UPLOAD_ERROR);
		resolve(upload.call(request.helper, [request.file], options));
	});
}

/** Starts one file's upload; `abort()` stops it and silences every later report. */
export function startUpload(request: UploadRequest): Upload {
	const controller = new AbortController();
	let live = true;
	const report = (input: SessionInput) => {
		if (live) request.report(input);
	};
	const run = async () => {
		try {
			const target =
				request.mode === "flowpath" ? await request.target() : null;
			if (!live) return;
			const results = await callHelper(
				request,
				target,
				controller.signal,
				progressReporter(request.slotId, report),
			);
			report(uploadInputOf(request.slotId, request.mode, results[0]));
		} catch (error) {
			if (controller.signal.aborted) return;
			report(failed(request.slotId, messageOf(error, "The upload failed.")));
		} finally {
			live = false;
		}
	};
	return {
		slotId: request.slotId,
		abort: () => {
			live = false;
			controller.abort();
		},
		done: run(),
	};
}

// ─── Inline files (hosted links, the device page) ──────────────────────────

/** A file of a run's copy that could not be put into the request. */
export class InlineFileError extends Error {
	constructor(
		readonly fileName: string,
		cause: unknown,
	) {
		super(`${fileName} could not be added to the run: ${messageOf(cause, "")}`);
		this.name = "InlineFileError";
	}
}

async function encodeSlot(
	slot: FileSlot,
	fileOf: (slotId: string) => File | undefined,
	helper: InlineEncoder,
): Promise<FileSlot> {
	if (slot.ref?.kind !== "inline") return slot;
	const file = fileOf(slot.id);
	if (!file) throw new InlineFileError(slot.name, NO_FILE_ERROR);
	try {
		const url = await new Promise<string>((resolve) =>
			resolve(helper.fileToUrl(file, false)),
		);
		if (!nonEmpty(url)) throw new Error(NO_URL_ERROR);
		return { ...slot, ref: { kind: "url", url } };
	} catch (error) {
		throw new InlineFileError(slot.name, error);
	}
}

async function encodeValue(
	value: CopyValue,
	fileOf: (slotId: string) => File | undefined,
	helper: InlineEncoder,
): Promise<CopyValue> {
	if (isFileSlot(value)) return encodeSlot(value, fileOf, helper);
	if (!Array.isArray(value) || !value.some(isFileSlot)) return value;
	return Promise.all(
		(value as readonly FileSlot[]).map((slot) =>
			encodeSlot(slot, fileOf, helper),
		),
	);
}

const holdsInline = (value: CopyValue | undefined) => {
	const items: readonly unknown[] = Array.isArray(value) ? value : [value];
	return items.some((item) => isFileSlot(item) && item.ref?.kind === "inline");
};

/**
 * A run's copy with every inline file replaced by its data: URL (`fileToUrl`), so `buildPayload`
 * sends it; the slots in the session keep their `inline` ref. Throws `InlineFileError` when a file
 * is gone or cannot be read.
 */
export async function encodeInlineValues(
	fields: readonly WorkbenchField[],
	values: Readonly<Record<string, CopyValue>>,
	fileOf: (slotId: string) => File | undefined,
	helper: InlineEncoder,
): Promise<Readonly<Record<string, CopyValue>>> {
	const names = fields
		.filter(isFileField)
		.map((field) => field.name)
		.filter((name) => holdsInline(values[name]));
	if (names.length === 0) return values;
	const encoded = await Promise.all(
		names.map((name) => encodeValue(values[name], fileOf, helper)),
	);
	const next: Record<string, CopyValue> = { ...values };
	names.forEach((name, index) => {
		next[name] = encoded[index];
	});
	return next;
}
