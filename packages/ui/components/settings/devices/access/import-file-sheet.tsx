"use client";

import { useTranslation } from "@flow-like/locales";
import { FileUp, Laptop } from "lucide-react";
import { useCallback, useState } from "react";
import {
	ACCESS_FILE_MAX_BYTES,
	type AccessFileError,
	type ImportedAccessRequest,
	parseAccessRequestFile,
	requestFileDevice,
} from "../../../../lib/device-management/sharing";
import type { Ed25519PublicKey } from "../../../../lib/device-management/types";
import { humanFileSize } from "../../../../lib/utils";
import type { DevicesT } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { DropZone } from "../primitives/form-fields";
import { InlineResult } from "../primitives/inline-result";
import { cx } from "../primitives/tone";
import { useGuardedRead } from "./use-access";

/** One person of one imported request file, or the file itself when it can't be used. */
export interface RequestFileRow {
	id: string;
	file: string;
	bytes: number;
	error?: { code: AccessFileError; count?: number };
	userId?: string;
	controllerKey?: Ed25519PublicKey;
	grantId?: string;
	/** From the file name, the importer's choice, or the single selected device. */
	deviceId?: string;
}

const newId = () => crypto.randomUUID();

async function readRequestFile(
	file: File,
	ownedIds: readonly string[],
	fallbackDeviceId: string | undefined,
): Promise<RequestFileRow[]> {
	const base = { file: file.name, bytes: file.size };
	if (file.size > ACCESS_FILE_MAX_BYTES)
		return [{ ...base, id: newId(), error: { code: "too_large" } }];
	let text: string;
	try {
		text = await file.text();
	} catch {
		return [{ ...base, id: newId(), error: { code: "not_request_file" } }];
	}
	const parsed = parseAccessRequestFile(text, file.size);
	if (!parsed.ok)
		return [
			{
				...base,
				id: newId(),
				error: {
					code: parsed.error,
					...(parsed.count === undefined ? {} : { count: parsed.count }),
				},
			},
		];
	const deviceId = requestFileDevice(file.name, ownedIds) ?? fallbackDeviceId;
	return parsed.recipients.map((recipient) => ({
		...base,
		id: newId(),
		userId: recipient.user_id,
		controllerKey: recipient.controller_key,
		grantId: recipient.grant_id ?? newId(),
		...(deviceId ? { deviceId } : {}),
	}));
}

/** Reads request files on this computer; nothing is uploaded. */
export async function readRequestFiles(
	files: readonly File[],
	ownedIds: readonly string[],
	fallbackDeviceId?: string,
): Promise<RequestFileRow[]> {
	const rows = await Promise.all(
		files.map((file) => readRequestFile(file, ownedIds, fallbackDeviceId)),
	);
	return rows.flat();
}

export function rowsOfRequests(
	requests: readonly ImportedAccessRequest[],
): RequestFileRow[] {
	return requests.map((request) => ({
		id: request.id,
		file: request.file,
		bytes: 0,
		userId: request.userId,
		controllerKey: request.controllerKey,
		...(request.grantId ? { grantId: request.grantId } : {}),
		...(request.deviceId ? { deviceId: request.deviceId } : {}),
	}));
}

/** The rows worth keeping under Access requests (files that could be read). */
export function requestsOfRows(
	rows: readonly RequestFileRow[],
	importedAt: number,
): ImportedAccessRequest[] {
	return rows.flatMap((row) =>
		row.error || !row.userId || !row.controllerKey
			? []
			: [
					{
						id: row.id,
						file: row.file,
						userId: row.userId,
						controllerKey: row.controllerKey,
						...(row.grantId ? { grantId: row.grantId } : {}),
						...(row.deviceId ? { deviceId: row.deviceId } : {}),
						importedAt,
					},
				],
	);
}

export function fileErrorText(t: DevicesT, row: RequestFileRow): string {
	switch (row.error?.code) {
		case "too_large":
			return t(
				"devices:access.files.error.tooLarge",
				"{{file}} is {{size}}. Access request files are at most 128 KiB.",
				{ file: row.file, size: humanFileSize(row.bytes) },
			);
		case "too_many_people":
			return t(
				"devices:access.files.error.tooMany",
				"It lists {{count, number}} people; access rules hold at most 24.",
				{ count: row.error.count ?? 0 },
			);
		case "incomplete_entry":
			return t(
				"devices:access.files.error.incomplete",
				"An entry is incomplete: each person needs an account ID (up to 128 characters) and a 43-character public key.",
			);
		default:
			return t(
				"devices:access.files.error.notRequest",
				"This isn't an access request file. It should list an account ID, a public key and a request ID.",
			);
	}
}

const REQUEST_ACCEPT = ".json,application/json";

/** Drop target for access request files; late reads never land in another account. */
export function RequestFileDrop({
	id,
	ownedIds,
	fallbackDeviceId,
	onRows,
	slim = false,
	className,
}: Readonly<{
	id: string;
	ownedIds: readonly string[];
	fallbackDeviceId?: string;
	onRows(rows: RequestFileRow[]): void;
	/** The one-line form inside the Access requests block. */
	slim?: boolean;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const guarded = useGuardedRead();
	const onFiles = useCallback(
		(files: File[]) => {
			void guarded(
				() => readRequestFiles(files, ownedIds, fallbackDeviceId),
				onRows,
			);
		},
		[guarded, ownedIds, fallbackDeviceId, onRows],
	);
	if (slim) return <SlimDrop id={id} onFiles={onFiles} className={className} />;
	return (
		<DropZone
			id={id}
			multiple
			accept={REQUEST_ACCEPT}
			icon={FileUp}
			title={
				<>
					<b className="font-semibold">
						{t("access.files.dropTitle", "Drop access request files")}
					</b>{" "}
					{t("access.files.dropOrChoose", "or choose them")}
				</>
			}
			hint={t(
				"access.files.hint",
				".json · up to 128 KiB each · several at once",
			)}
			onFiles={onFiles}
			className={className}
		/>
	);
}

/** The one-row drop target of the Access requests block (the `DropZone` primitive has only the tall form). */
function SlimDrop({
	id,
	onFiles,
	className,
}: Readonly<{ id: string; onFiles(files: File[]): void; className?: string }>) {
	const { t } = useTranslation("devices");
	const [over, setOver] = useState(false);
	return (
		<div className={className}>
			<label
				htmlFor={id}
				data-over={over || undefined}
				onDragOver={(event) => {
					event.preventDefault();
					setOver(true);
				}}
				onDragLeave={() => setOver(false)}
				onDrop={(event) => {
					event.preventDefault();
					setOver(false);
					const dropped = Array.from(event.dataTransfer?.files ?? []);
					if (dropped.length) onFiles(dropped);
				}}
				className={cx(
					"flex cursor-pointer flex-wrap items-center justify-center gap-x-2.5 gap-y-1.5 rounded-lg border border-dashed border-border-strong bg-surface-sunken px-4 py-3.5 text-center text-sm/5 text-ink-2 hover:border-foreground hover:bg-row-hover focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ring",
					over && "border-foreground bg-row-hover",
				)}
			>
				<FileUp aria-hidden className="size-5 text-muted-foreground" />
				<span>
					<b className="font-semibold text-foreground">
						{t("access.files.importTitle", "Import request files")}
					</b>{" "}
					<span className="text-muted-foreground">
						{t("access.files.importOrDrop", "or drop them here")}
					</span>
				</span>
				<span className="text-xs text-muted-foreground">
					{t("access.files.hintSlim", ".json · up to 128 KiB each")}
				</span>
				<input
					id={id}
					type="file"
					accept={REQUEST_ACCEPT}
					multiple
					className="sr-only"
					onChange={(event) => {
						const chosen = Array.from(event.target.files ?? []);
						if (chosen.length) onFiles(chosen);
						event.target.value = "";
					}}
				/>
			</label>
		</div>
	);
}

/**
 * `import=request` and "Import request files": pick the files a person sent
 * you. What can be read is kept under Access requests and handed on to
 * Add people.
 */
export function ImportFileSheet({
	open,
	onOpenChange,
	ownedIds,
	onImported,
}: Readonly<{
	open: boolean;
	onOpenChange(open: boolean): void;
	ownedIds: readonly string[];
	/** Every row read, usable or not; the caller keeps the usable ones and opens the review. */
	onImported(rows: RequestFileRow[]): void;
}>) {
	const { t } = useTranslation("devices");
	const [unusable, setUnusable] = useState<RequestFileRow[]>([]);
	const close = (next: boolean) => {
		if (!next) setUnusable([]);
		onOpenChange(next);
	};
	const onRows = useCallback(
		(rows: RequestFileRow[]) => {
			if (rows.some((row) => !row.error)) {
				setUnusable([]);
				onImported(rows);
				return;
			}
			setUnusable(rows);
		},
		[onImported],
	);
	return (
		<DvSheet
			open={open}
			onOpenChange={close}
			icon={FileUp}
			title={t("access.files.sheetTitle", "Import access request files")}
			sub={t(
				"access.files.sheetSub",
				"A person sends you this file after importing your device's connection file",
			)}
			foot={
				<DvButton onClick={() => close(false)}>
					{t("access.files.cancel", "Cancel")}
				</DvButton>
			}
		>
			<p className="text-sm/5">
				{t(
					"access.files.sheetIntro",
					"Each file names one account, the public key their app made for one of your devices and a request ID. Nothing in it is secret. After importing you compare the key fingerprint with the person and choose their permissions.",
				)}
			</p>
			<RequestFileDrop
				id="access-import-files"
				ownedIds={ownedIds}
				onRows={onRows}
			/>
			<p className="flex items-center gap-1.5 text-xs text-muted-foreground">
				<Laptop aria-hidden className="size-3.25" />
				{t("access.files.local", "Read on this computer. Nothing is uploaded.")}
			</p>
			{unusable.map((row) => (
				<InlineResult key={row.id} tone="critical">
					{fileErrorText(t, row)}
				</InlineResult>
			))}
		</DvSheet>
	);
}
