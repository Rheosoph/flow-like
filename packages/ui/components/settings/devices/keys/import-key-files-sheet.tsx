"use client";

import { useTranslation } from "@flow-like/locales";
import { FileText, FileUp, ShieldCheck } from "lucide-react";
import { useRef, useState } from "react";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { DropZone, SecretInput, utf8Bytes } from "../primitives/form-fields";
import { InlineResult } from "../primitives/inline-result";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { RESTORE_RESULT } from "./account-backup-panel";
import { classifyKeyFailure, keyFailureCopy } from "./key-errors";
import {
	KEY_FILE_MAX_BYTES,
	PASSWORD_MIN_BYTES,
	type ParsedKeyFile,
	parseKeyFile,
} from "./key-operations";
import {
	Mono,
	RowStatus,
	type RowStatusTone,
	SheetList,
	SheetListItem,
	keyKindLabel,
	useNames,
} from "./key-parts";
import { rowScope, useKeyResults } from "./key-store";
import { useFlowGuard, useKeyActions } from "./use-key-actions";
import type { KeysRead } from "./use-keys-model";

interface ImportEntry {
	id: string;
	fileName: string;
	/** The file can't be used; kept in the list so the reason stays visible. */
	error?: string;
	parsed?: ParsedKeyFile;
	deviceName?: string;
	role?: "owner" | "shared";
	password: string;
	status?: { tone: RowStatusTone; text: string };
}

type ImportPhase = "form" | "progress" | "done";

const isSealed = (entry: ImportEntry) => entry.parsed?.file.sealed === true;

/** SPEC §5.9 "Import backup files…": several files at once, one password per sealed file. */
export function ImportKeyFilesSheet({
	read,
	onClose,
}: Readonly<{ read: KeysRead; onClose(): void }>) {
	const { t } = useTranslation("devices");
	const names = useNames();
	const workspace = useDeviceWorkspace();
	const actions = useKeyActions();
	const results = useKeyResults();
	const guard = useFlowGuard();
	const working = useRef(false);
	const [entries, setEntries] = useState<ImportEntry[]>([]);
	const [phase, setPhase] = useState<ImportPhase>("form");
	const [error, setError] = useState<string>();
	const [summary, setSummary] = useState<{
		tone: "good" | "warning";
		text: string;
	}>();
	const latest = useRef(read);
	latest.current = read;

	const describe = async (file: File): Promise<ImportEntry> => {
		const base = { id: crypto.randomUUID(), fileName: file.name, password: "" };
		const refuse = (failure: unknown): ImportEntry => ({
			...base,
			error: keyFailureCopy(
				t,
				classifyKeyFailure(failure, "not_a_backup"),
				file.name,
			),
		});
		if (file.size > KEY_FILE_MAX_BYTES)
			return {
				...base,
				error: keyFailureCopy(t, { code: "too_large" }, file.name),
			};
		try {
			const parsed = parseKeyFile(await file.text(), workspace.deps.scope);
			const { model, devices } = latest.current;
			const row = model.rows.find(
				(entry) => entry.deviceId === parsed.deviceId,
			);
			if (!row && devices.loaded)
				return {
					...base,
					error: keyFailureCopy(t, { code: "other_device" }, file.name),
				};
			const role = parsed.file.sealed
				? row?.relationship === "owner"
					? "owner"
					: "shared"
				: "owner";
			return {
				...base,
				parsed,
				deviceName: row?.name ?? parsed.deviceId.slice(0, 8),
				role,
			};
		} catch (failure) {
			return refuse(failure);
		}
	};

	const add = async (files: File[]) => {
		const flow = guard();
		const described = await Promise.all(files.map(describe));
		if (!flow.alive()) return;
		setError(undefined);
		setEntries((current) => [
			...current,
			...described.filter(
				(entry) => !current.some((row) => row.fileName === entry.fileName),
			),
		]);
	};

	const patch = (id: string, change: Partial<ImportEntry>) =>
		setEntries((current) =>
			current.map((entry) =>
				entry.id === id ? { ...entry, ...change } : entry,
			),
		);

	const valid = entries.filter((entry) => entry.parsed && !entry.status);

	const run = async () => {
		if (working.current || phase !== "form" || !valid.length) return;
		const short = valid.filter(
			(entry) =>
				isSealed(entry) && utf8Bytes(entry.password) < PASSWORD_MIN_BYTES,
		);
		if (short.length) {
			setError(
				t(
					"keys.import.missing",
					"Enter the device password for {{devices}}. Device passwords are at least 12 bytes.",
					{ devices: names(short.map((entry) => entry.deviceName ?? "")) },
				),
			);
			return;
		}
		const flow = guard();
		const jobs = valid.map((entry) => ({ entry, secret: entry.password }));
		working.current = true;
		setEntries((current) =>
			current.map((entry) => ({ ...entry, password: "" })),
		);
		setError(undefined);
		setPhase("progress");
		const imported: string[] = [];
		const same: string[] = [];
		const failed: string[] = [];
		for (const { entry, secret } of jobs) {
			const parsed = entry.parsed as ParsedKeyFile;
			const device = entry.deviceName ?? parsed.deviceId;
			patch(entry.id, {
				status: {
					tone: "info",
					text: t("keys.import.verifying", "Verifying backup…"),
				},
			});
			const outcome = await actions.importFile(
				{ deviceId: parsed.deviceId, name: device },
				parsed,
				secret,
			);
			if (!flow.alive()) return;
			if (!outcome.ok) {
				failed.push(device);
				patch(entry.id, { status: { tone: "critical", text: outcome.text } });
				continue;
			}
			if (outcome.value === "same") {
				same.push(device);
				patch(entry.id, {
					status: {
						tone: "good",
						text: t(
							"keys.import.rowSame",
							"{{device}} already here · nothing changed",
							{ device },
						),
					},
				});
				continue;
			}
			imported.push(device);
			patch(entry.id, {
				status: {
					tone: "good",
					text: t("keys.import.rowDone", "Imported for {{device}} · locked", {
						device,
					}),
				},
			});
			results.put(
				rowScope(parsed.deviceId),
				"good",
				t(
					"keys.result.imported",
					"Imported from {{file}} at {{at}}. Unlock with the device password to use the keys.",
					{ file: entry.fileName, at: actions.timeNow() },
				),
			);
		}
		working.current = false;
		const refused = entries.filter((entry) => entry.error).length;
		const parts = [
			imported.length
				? t("keys.import.summaryImported", "Imported keys for {{devices}}.", {
						devices: names(imported),
					})
				: "",
			same.length
				? t(
						"keys.import.summarySame",
						"{{devices}} already had the same keys here.",
						{ devices: names(same) },
					)
				: "",
			failed.length
				? t("keys.import.summaryFailed", "Not imported: {{devices}}.", {
						devices: names(failed),
					})
				: "",
			refused
				? t("keys.import.summaryRefused", {
						count: refused,
						defaultValue_one: "{{count, number}} file couldn't be used.",
						defaultValue_other: "{{count, number}} files couldn't be used.",
					})
				: "",
		].filter(Boolean);
		const text =
			parts.join(" ") || t("keys.import.summaryNone", "Nothing was imported.");
		const tone = failed.length || refused ? "warning" : "good";
		setSummary({ tone, text });
		results.put(
			RESTORE_RESULT,
			tone,
			t("keys.import.topResult", "Import finished at {{at}}. {{summary}}", {
				at: actions.timeNow(),
				summary: text,
			}),
		);
		setPhase("done");
	};

	const close = () => {
		setEntries([]);
		onClose();
	};

	return (
		<DvSheet
			open
			wide
			closeOnOutside={phase !== "progress"}
			onOpenChange={(open) => {
				if (!open && phase !== "progress") close();
			}}
			icon={FileUp}
			title={t("keys.import.title", "Import key backup files")}
			sub={t(
				"keys.import.subtitle",
				"Bring keys from files you saved, one file per device.",
			)}
			footNote={
				phase === "done"
					? t("keys.import.noteDone", "Imported keys start locked.")
					: t(
							"keys.import.noteForm",
							"Files with errors are kept in the list so you can see why.",
						)
			}
			foot={
				phase === "done" ? (
					<DvButton variant="primary" onClick={close}>
						{t("keys.sheet.done", "Done")}
					</DvButton>
				) : (
					<>
						<DvButton onClick={close} aria-disabled={phase === "progress"}>
							{t("keys.sheet.cancel", "Cancel")}
						</DvButton>
						<DvButton
							variant="primary"
							icon={ShieldCheck}
							busy={phase === "progress"}
							aria-disabled={!valid.length || undefined}
							onClick={() => void run()}
						>
							{valid.length
								? t("keys.import.go", {
										count: valid.length,
										defaultValue_one:
											"Verify and import {{count, number}} file",
										defaultValue_other:
											"Verify and import {{count, number}} files",
									})
								: t("keys.import.goNone", "Verify and import")}
						</DvButton>
					</>
				)
			}
		>
			<p className="text-sm">
				{t(
					"keys.import.intro",
					"A key backup file opens only with the device password it was saved with. Importing checks the file, then keeps the keys on this computer, locked.",
				)}
			</p>
			{phase === "form" ? (
				<DropZone
					id="keys-import-files"
					multiple
					accept=".json,application/json"
					title={
						<>
							<b className="font-semibold">
								{t("keys.import.drop", "Drop key backup files")}
							</b>{" "}
							{t("keys.import.dropOr", "or choose them")}
						</>
					}
					hint={t(
						"keys.import.dropHint",
						".json · up to 1 MiB each · several at once",
					)}
					onFiles={(files) => void add(files)}
				/>
			) : null}
			{entries.length ? (
				<SheetList>
					{entries.map((entry) => {
						const device = entry.deviceName ?? "";
						return (
							<SheetListItem
								key={entry.id}
								data-import-file={entry.fileName}
								head={
									<span
										title={entry.fileName}
										className="flex min-w-0 max-w-full items-center gap-1 font-mono text-xs text-ink-2"
									>
										<FileText aria-hidden className="size-3 shrink-0" />
										<span className="truncate">{entry.fileName}</span>
									</span>
								}
								status={
									entry.status ? (
										<RowStatus tone={entry.status.tone}>
											{entry.status.text}
										</RowStatus>
									) : entry.error ? (
										<RowStatus tone="critical">
											{t("keys.import.cant", "Can't import")}
										</RowStatus>
									) : (
										<RowStatus tone="muted">
											{t("keys.import.for", "{{kind}} for", {
												kind: keyKindLabel(t, entry.role ?? "owner"),
											})}{" "}
											<Mono>{device}</Mono>
										</RowStatus>
									)
								}
							>
								{entry.error ? (
									<p className="text-xs text-critical">{entry.error}</p>
								) : null}
								{entry.parsed && !entry.status && phase === "form" ? (
									isSealed(entry) ? (
										<SecretInput
											value={entry.password}
											onValueChange={(password) =>
												patch(entry.id, { password })
											}
											aria-label={t(
												"keys.import.password",
												"Password of the backup for {{device}}",
												{ device },
											)}
											placeholder={t(
												"keys.field.devicePassword",
												"Device password for {{device}}",
												{ device },
											)}
										/>
									) : (
										<p className="text-xs text-muted-foreground">
											{t(
												"keys.import.legacy",
												"Older file format: it is imported as it is and still opens only with its device password.",
											)}
										</p>
									)
								) : null}
							</SheetListItem>
						);
					})}
				</SheetList>
			) : null}
			{error ? <InlineResult tone="critical">{error}</InlineResult> : null}
			{phase === "done" && summary ? (
				<InlineResult tone={summary.tone}>{summary.text}</InlineResult>
			) : null}
		</DvSheet>
	);
}
