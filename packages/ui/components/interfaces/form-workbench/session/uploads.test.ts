import { describe, expect, test } from "bun:test";
import type { ITemporaryUploadResult } from "../../../../lib/temporary-upload-batch";
import type {
	IHelperState,
	ITemporaryUploadedFile,
} from "../../../../state/backend-state/helper-state";
import {
	type CopyValue,
	type ExecutionTarget,
	type FileSlot,
	REQUEST_FILES_STORE_REF,
	type SessionInput,
	type WorkbenchField,
} from "../contracts";
import { FIXTURE_FIELDS } from "../testing/fixtures";
import {
	InlineFileError,
	NO_FLOW_PATH_ERROR,
	NO_UPLOAD_ERROR,
	NO_URL_ERROR,
	type UploadHelper,
	encodeInlineValues,
	expiresAtOf,
	flowPathOf,
	startUpload,
	uploadInputOf,
} from "./uploads";

type UploadOptions = NonNullable<
	Parameters<NonNullable<IHelperState["filesToTemporaryFiles"]>>[1]
>;
type Answer = (
	file: File,
	options: UploadOptions,
) => Promise<ITemporaryUploadResult[]>;

const FLOW_PATH = {
	path: "tmp/global/apps/app/events/event/requests/r1/0001-invoice.pdf",
	store_ref: REQUEST_FILES_STORE_REF,
	cache_store_ref: null,
};

const pdf = () =>
	new File(["%PDF-1.7"], "invoice.pdf", { type: "application/pdf" });

function helperWith(answer: Answer) {
	const calls: { files: File[]; options: UploadOptions }[] = [];
	const helper: UploadHelper = {
		filesToTemporaryFiles: (files, options) => {
			const opts = options ?? {};
			calls.push({ files, options: opts });
			return answer(files[0], opts);
		},
	};
	return { helper, calls };
}

const uploaded = (file: File, result: ITemporaryUploadedFile) => [
	{ file, uploaded: result },
];

function upload(
	helper: UploadHelper,
	mode: "flowpath" | "url",
	target: () => Promise<ExecutionTarget> = async () => "local",
) {
	const reports: SessionInput[] = [];
	const running = startUpload({
		slotId: "slot-1",
		file: pdf(),
		mode,
		appId: "app",
		eventId: "event",
		target,
		helper,
		report: (input) => reports.push(input),
	});
	return { reports, running };
}

describe("startUpload", () => {
	test("a FlowPath field uploads for its app, event and target and keeps the FlowPath", async () => {
		const { helper, calls } = helperWith(async (file, options) => {
			options.onProgress?.(50);
			options.onProgress?.(100);
			return uploaded(file, { url: "", flowPath: FLOW_PATH });
		});
		const { reports, running } = upload(helper, "flowpath");
		await running.done;

		expect(calls).toHaveLength(1);
		expect(calls[0].files.map((file) => file.name)).toEqual(["invoice.pdf"]);
		const { signal, onProgress, ...rest } = calls[0].options;
		expect(rest).toEqual({
			appId: "app",
			eventId: "event",
			executionTarget: "local",
		});
		expect(signal).toBeInstanceOf(AbortSignal);
		expect(typeof onProgress).toBe("function");
		expect(reports).toEqual([
			{ type: "uploadProgress", slotId: "slot-1", progress: 0.5 },
			{ type: "uploadProgress", slotId: "slot-1", progress: 1 },
			{
				type: "uploadSent",
				slotId: "slot-1",
				ref: { kind: "flowpath", flowPath: FLOW_PATH, url: null },
				expiresAt: null,
			},
		]);
	});

	test("a FlowPath upload that comes back without one fails, never falls back to its URL", async () => {
		const { helper } = helperWith(async (file) =>
			uploaded(file, { url: "https://files.test/invoice.pdf" }),
		);
		const { reports, running } = upload(helper, "flowpath");
		await running.done;
		expect(reports).toEqual([
			{ type: "uploadFailed", slotId: "slot-1", error: NO_FLOW_PATH_ERROR },
		]);
	});

	test("a legacy field uploads without app, event and target and keeps the URL until it expires", async () => {
		const { helper, calls } = helperWith(async (file) =>
			uploaded(file, {
				url: "https://files.test/invoice.pdf",
				flowPath: FLOW_PATH,
				downloadExpiresAt: "2026-10-12T10:00:00.000Z",
			}),
		);
		const { reports, running } = upload(helper, "url");
		await running.done;

		expect(Object.keys(calls[0].options).sort()).toEqual([
			"onProgress",
			"signal",
		]);
		expect(reports).toEqual([
			{
				type: "uploadSent",
				slotId: "slot-1",
				ref: { kind: "url", url: "https://files.test/invoice.pdf" },
				expiresAt: Date.parse("2026-10-12T10:00:00.000Z"),
			},
		]);
	});

	test("a FlowPath upload waits for the run target; a legacy one does not", async () => {
		let resolveTarget: (target: ExecutionTarget) => void = () => {};
		const target = new Promise<ExecutionTarget>((resolve) => {
			resolveTarget = resolve;
		});
		const flow = helperWith(async (file) =>
			uploaded(file, { url: "", flowPath: FLOW_PATH }),
		);
		const legacy = helperWith(async (file) =>
			uploaded(file, { url: "https://files.test/a" }),
		);
		const waiting = upload(flow.helper, "flowpath", () => target);
		const direct = upload(legacy.helper, "url", () => target);
		await direct.running.done;
		expect(flow.calls).toHaveLength(0);
		expect(legacy.calls).toHaveLength(1);

		resolveTarget("remote");
		await waiting.running.done;
		expect(flow.calls[0].options.executionTarget).toBe("remote");
	});

	test("a file the upload refused fails with the upload's own words", async () => {
		const { helper } = helperWith(async (file) => [
			{ file, error: "Upload failed (403 Forbidden)" },
		]);
		const { reports, running } = upload(helper, "url");
		await running.done;
		expect(reports).toEqual([
			{
				type: "uploadFailed",
				slotId: "slot-1",
				error: "Upload failed (403 Forbidden)",
			},
		]);
	});

	test("a helper that throws, even synchronously, fails the file", async () => {
		const throwing: UploadHelper = {
			filesToTemporaryFiles: () => {
				throw new Error("Profile or auth not set");
			},
		};
		const { reports, running } = upload(throwing, "url");
		await running.done;
		expect(reports).toEqual([
			{
				type: "uploadFailed",
				slotId: "slot-1",
				error: "Profile or auth not set",
			},
		]);

		const missing = upload({}, "url");
		await missing.running.done;
		expect(missing.reports).toEqual([
			{ type: "uploadFailed", slotId: "slot-1", error: NO_UPLOAD_ERROR },
		]);
	});

	test("abort stops the transfer and nothing is reported afterwards", async () => {
		let seen: AbortSignal | undefined;
		const { helper } = helperWith(
			(_file, options) =>
				new Promise((_resolve, reject) => {
					seen = options.signal;
					options.onProgress?.(20);
					options.signal?.addEventListener("abort", () =>
						reject(new DOMException("Aborted", "AbortError")),
					);
				}),
		);
		const { reports, running } = upload(helper, "url");
		await Promise.resolve();
		running.abort();
		await running.done;
		expect(seen?.aborted).toBe(true);
		expect(reports).toEqual([
			{ type: "uploadProgress", slotId: "slot-1", progress: 0.2 },
		]);
	});

	test("a result that arrives after an abort is dropped", async () => {
		let finish: () => void = () => {};
		const { helper } = helperWith(
			(file) =>
				new Promise((resolve) => {
					finish = () => resolve(uploaded(file, { url: "https://x.test/a" }));
				}),
		);
		const { reports, running } = upload(helper, "url");
		await Promise.resolve();
		running.abort();
		finish();
		await running.done;
		expect(reports).toEqual([]);
	});

	test("progress is reported as 0–1 and only when it moved by a percent", async () => {
		const { helper } = helperWith(async (file, options) => {
			for (const percent of [10, 10.5, 30, 30, 100, 100])
				options.onProgress?.(percent);
			return uploaded(file, { url: "https://x.test/a" });
		});
		const { reports, running } = upload(helper, "url");
		await running.done;
		expect(
			reports
				.filter((input) => input.type === "uploadProgress")
				.map((input) => (input.type === "uploadProgress" ? input.progress : 0)),
		).toEqual([0.1, 0.3, 1]);
	});
});

describe("upload results", () => {
	test("a FlowPath needs a path and a store; an empty cache store is none", () => {
		expect(flowPathOf({ path: "", store_ref: "s" })).toBeNull();
		expect(flowPathOf({ path: "p", store_ref: "" })).toBeNull();
		expect(flowPathOf(undefined)).toBeNull();
		expect(
			flowPathOf({ path: "p", store_ref: "s", cache_store_ref: "" }),
		).toEqual({
			path: "p",
			store_ref: "s",
			cache_store_ref: null,
		});
	});

	test("a URL upload without an address fails; a missing result fails", () => {
		expect(
			uploadInputOf("s", "url", { file: pdf(), uploaded: { url: "" } }),
		).toEqual({ type: "uploadFailed", slotId: "s", error: NO_URL_ERROR });
		expect(uploadInputOf("s", "url", undefined)).toEqual({
			type: "uploadFailed",
			slotId: "s",
			error: "The upload failed.",
		});
	});

	test("expiry times read as ms since the epoch", () => {
		expect(expiresAtOf("2026-10-12T10:00:00Z")).toBe(
			Date.UTC(2026, 9, 12, 10, 0, 0),
		);
		expect(expiresAtOf("soon")).toBeNull();
		expect(expiresAtOf(undefined)).toBeNull();
	});
});

// ─── Inline files ───────────────────────────────────────────────────────────

const receipt = FIXTURE_FIELDS.small[2];
const ATTACHMENT: WorkbenchField = {
	...receipt,
	key: "attachment",
	name: "attachment",
	label: "Attachment",
	dataType: "PathBuf",
	fileMode: "url",
};
const DOCUMENTS: WorkbenchField = {
	...ATTACHMENT,
	key: "documents",
	name: "documents",
	label: "Documents",
	kind: "files",
	valueType: "Array",
};
const ORDER = FIXTURE_FIELDS.small[0];

function slot(id: string, ref: FileSlot["ref"]): FileSlot {
	return {
		id,
		name: `${id}.txt`,
		size: 5,
		type: "text/plain",
		state: "sent",
		progress: null,
		ref,
		error: null,
		sentAt: 1,
		expiresAt: null,
	};
}

const INLINE = { kind: "inline" } as const;

describe("encodeInlineValues", () => {
	const files = new Map<string, File>([
		["a", new File(["alpha"], "a.txt", { type: "text/plain" })],
		["b", new File(["beta"], "b.txt", { type: "text/plain" })],
	]);
	const encoder = {
		fileToUrl: async (file: File) =>
			`data:text/plain;base64,${btoa(await file.text())}`,
	};

	test("inline files become data: URLs; the session's slots stay inline", async () => {
		const values: Readonly<Record<string, CopyValue>> = {
			order: "A-1",
			attachment: slot("a", INLINE),
			documents: [
				slot("b", INLINE),
				slot("c", { kind: "url", url: "https://x.test/c" }),
			],
		};
		const encoded = await encodeInlineValues(
			[ORDER, ATTACHMENT, DOCUMENTS],
			values,
			(id) => files.get(id),
			encoder,
		);
		expect(encoded.order).toBe("A-1");
		expect((encoded.attachment as FileSlot).ref).toEqual({
			kind: "url",
			url: `data:text/plain;base64,${btoa("alpha")}`,
		});
		expect((encoded.documents as FileSlot[]).map((item) => item.ref)).toEqual([
			{ kind: "url", url: `data:text/plain;base64,${btoa("beta")}` },
			{ kind: "url", url: "https://x.test/c" },
		]);
		expect((values.attachment as FileSlot).ref).toEqual(INLINE);
	});

	test("a copy without inline files is passed through as it is", async () => {
		const values = {
			attachment: slot("c", { kind: "url", url: "https://x.test/c" }),
		};
		const encoded = await encodeInlineValues(
			[ATTACHMENT],
			values,
			() => undefined,
			encoder,
		);
		expect(encoded).toBe(values);
	});

	test("a file that is gone or cannot be read is named in the error", async () => {
		const gone = encodeInlineValues(
			[ATTACHMENT],
			{ attachment: slot("missing", INLINE) },
			(id) => files.get(id),
			encoder,
		);
		await expect(gone).rejects.toBeInstanceOf(InlineFileError);
		await expect(gone).rejects.toMatchObject({ fileName: "missing.txt" });

		const unreadable = encodeInlineValues(
			[ATTACHMENT],
			{ attachment: slot("a", INLINE) },
			(id) => files.get(id),
			{
				fileToUrl: async () => {
					throw new Error("Attachments must be smaller than 20 MB.");
				},
			},
		);
		await expect(unreadable).rejects.toMatchObject({
			fileName: "a.txt",
			message:
				"a.txt could not be added to the run: Attachments must be smaller than 20 MB.",
		});
	});
});
