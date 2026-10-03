"use client";

import { useTranslation } from "@flow-like/locales";
import { Download, LockOpen } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { evaluateGate } from "../../../../lib/device-management/model/gates";
import { useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { CheckField, SecretInput, utf8Bytes } from "../primitives/form-fields";
import { type Gate, GateInline } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { useOverlay } from "../workspace/overlay-store";
import {
	buildGateContext,
	useAttentionState,
} from "../workspace/use-attention";
import { PASSWORD_MIN_BYTES, keyFileName } from "./key-operations";
import {
	Mono,
	RowStatus,
	type RowStatusTone,
	SamePassword,
	SheetList,
	SheetListItem,
	keyKindLabel,
	useNames,
} from "./key-parts";
import { type KeyRow, rowsWithKeys } from "./keys-model";
import { gateOf, useFlowGuard, useKeyActions } from "./use-key-actions";
import type { KeysRead } from "./use-keys-model";

interface RowState {
	tone: RowStatusTone;
	text: string;
}

function DownloadRow({
	row,
	checked,
	onChecked,
	password,
	onPassword,
	showPassword,
	state,
	locked,
	gate,
	onUnlock,
}: Readonly<{
	row: KeyRow;
	checked: boolean;
	onChecked(checked: boolean): void;
	password: string;
	onPassword(password: string): void;
	showPassword: boolean;
	state?: RowState;
	locked: boolean;
	gate: Gate | null;
	/** Sheets never stack: the unlock sheet replaces this one. */
	onUnlock(): void;
}>) {
	const { t } = useTranslation("devices");
	const gated = gate !== null;
	const role = row.vault?.role ?? "owner";
	const device = row.name;
	return (
		<SheetListItem
			data-download-row={row.deviceId}
			head={
				<CheckField
					id={`keys-download-${row.deviceId}`}
					checked={checked && !gated}
					disabled={gated || locked}
					onCheckedChange={onChecked}
				>
					<Mono>{device}</Mono>
				</CheckField>
			}
			status={
				state ? (
					<RowStatus tone={state.tone}>{state.text}</RowStatus>
				) : (
					<RowStatus tone="muted">{keyKindLabel(t, role)}</RowStatus>
				)
			}
		>
			<span
				title={keyFileName(row.deviceId)}
				className="block truncate font-mono text-xs text-ink-2"
			>
				{keyFileName(row.deviceId)}
			</span>
			{gate ? (
				<div className="flex flex-wrap items-center gap-2">
					<GateInline kind={gate.kind} className="max-w-none">
						{gate.reason}
					</GateInline>
					{gate.kind === "locked" ? (
						<DvButton size="xs" icon={LockOpen} onClick={onUnlock}>
							{t("keys.download.unlock", "Unlock…")}
						</DvButton>
					) : null}
				</div>
			) : showPassword && checked ? (
				<SecretInput
					value={password}
					onValueChange={onPassword}
					aria-label={t(
						"keys.field.devicePassword",
						"Device password for {{device}}",
						{ device },
					)}
					placeholder={t(
						"keys.field.devicePassword",
						"Device password for {{device}}",
						{ device },
					)}
				/>
			) : null}
		</SheetListItem>
	);
}

/** "Download backup files…": one sealed file per device, a second copy the user keeps. */
export function DownloadKeyFilesSheet({
	read,
	preselect,
	onClose,
}: Readonly<{ read: KeysRead; preselect?: string; onClose(): void }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const names = useNames();
	const actions = useKeyActions();
	const guard = useFlowGuard();
	const overlay = useOverlay();
	const working = useRef(false);
	const candidates = rowsWithKeys(read.model);
	const [selected, setSelected] = useState<Record<string, boolean>>(() =>
		Object.fromEntries(
			candidates.map((row) => [
				row.deviceId,
				!preselect || preselect === row.deviceId,
			]),
		),
	);
	const sources = useAttentionState();
	const ids = candidates.map((row) => row.deviceId).join("|");
	// biome-ignore lint/correctness/useExhaustiveDependencies: `ids` stands for the candidate list
	const gates = useMemo(
		() =>
			Object.fromEntries(
				candidates.map((row) => [
					row.deviceId,
					gateOf(
						evaluateGate(
							"download_key_file",
							buildGateContext(sources, row.deviceId),
						),
						t,
						time,
					),
				]),
			),
		[ids, sources, t, time],
	);
	const [passwords, setPasswords] = useState<Record<string, string>>({});
	const [same, setSame] = useState(false);
	const [samePassword, setSamePassword] = useState("");
	const [busy, setBusy] = useState(false);
	const [states, setStates] = useState<Record<string, RowState>>({});
	const [error, setError] = useState<string>();
	const [done, setDone] = useState<{
		tone: "good" | "warning";
		text: string;
	}>();

	const chosen = candidates.filter(
		(row) => selected[row.deviceId] && !gates[row.deviceId],
	);
	const passwordOf = (row: KeyRow) =>
		same ? samePassword : (passwords[row.deviceId] ?? "");
	const single = preselect
		? candidates.find((row) => row.deviceId === preselect)
		: undefined;

	const run = async () => {
		if (working.current || !chosen.length) return;
		const short = chosen.filter(
			(row) => utf8Bytes(passwordOf(row)) < PASSWORD_MIN_BYTES,
		);
		if (short.length) {
			setError(
				t(
					"keys.download.missing",
					"Enter the device password for {{devices}}. Device passwords are at least 12 bytes.",
					{ devices: names(short.map((row) => row.name)) },
				),
			);
			return;
		}
		const flow = guard();
		const jobs = chosen.map((row) => ({ row, secret: passwordOf(row) }));
		working.current = true;
		setPasswords({});
		setSamePassword("");
		setError(undefined);
		setBusy(true);
		const saved: string[] = [];
		const failed: string[] = [];
		for (const { row, secret } of jobs) {
			const outcome = await actions.downloadFile(
				{ deviceId: row.deviceId, name: row.name },
				secret,
			);
			if (!flow.alive()) return;
			if (outcome.ok) saved.push(outcome.value);
			else failed.push(row.name);
			setStates((current) => ({
				...current,
				[row.deviceId]: outcome.ok
					? { tone: "good", text: t("keys.download.rowDone", "Saved") }
					: { tone: "critical", text: outcome.text },
			}));
		}
		working.current = false;
		setBusy(false);
		const at = actions.timeNow();
		const text = [
			saved.length
				? t("keys.download.summary", {
						count: saved.length,
						at,
						defaultValue_one:
							"{{count, number}} backup file was saved at {{at}}. It opens only with the device password.",
						defaultValue_other:
							"{{count, number}} backup files were saved at {{at}}. Each opens only with its device password.",
					})
				: "",
			failed.length
				? t("keys.download.summaryFailed", "Not saved: {{devices}}.", {
						devices: names(failed),
					})
				: "",
		]
			.filter(Boolean)
			.join(" ");
		if (saved.length)
			setDone({ tone: failed.length ? "warning" : "good", text });
	};

	const close = () => {
		setPasswords({});
		setSamePassword("");
		onClose();
	};

	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open && !busy) close();
			}}
			icon={Download}
			title={
				single
					? t(
							"keys.download.titleOne",
							"Download the key backup file for {{device}}",
							{ device: single.name },
						)
					: t("keys.download.title", "Download key backup files")
			}
			sub={t("keys.download.subtitle", "Sealed files you keep yourself")}
			footNote={
				done
					? undefined
					: t("keys.download.note", "Files open only with the device password.")
			}
			foot={
				done ? (
					<DvButton variant="primary" onClick={close}>
						{t("keys.sheet.done", "Done")}
					</DvButton>
				) : (
					<>
						<DvButton onClick={close} aria-disabled={busy || undefined}>
							{t("keys.sheet.cancel", "Cancel")}
						</DvButton>
						<DvButton
							variant="primary"
							icon={Download}
							busy={busy}
							aria-disabled={!chosen.length || undefined}
							onClick={() => void run()}
						>
							{t("keys.download.go", {
								count: chosen.length,
								defaultValue_one: "Download {{count, number}} file",
								defaultValue_other: "Download {{count, number}} files",
							})}
						</DvButton>
					</>
				)
			}
		>
			<p className="text-sm">
				{t(
					"keys.download.intro",
					"Each file holds one device's keys, sealed with that device's current password. It's a second copy you keep yourself, for example on a USB stick or in a password manager.",
				)}
			</p>
			<SheetList>
				{candidates.map((row) => (
					<DownloadRow
						key={row.deviceId}
						row={row}
						checked={Boolean(selected[row.deviceId])}
						onChecked={(checked) =>
							setSelected((current) => ({
								...current,
								[row.deviceId]: checked,
							}))
						}
						password={passwords[row.deviceId] ?? ""}
						onPassword={(value) =>
							setPasswords((current) => ({
								...current,
								[row.deviceId]: value,
							}))
						}
						showPassword={!same && !done && !busy}
						state={states[row.deviceId]}
						locked={busy || done !== undefined}
						gate={gates[row.deviceId] ?? null}
						onUnlock={() => {
							close();
							overlay.openUnlock(row.deviceId);
						}}
					/>
				))}
			</SheetList>
			{!done && !busy && chosen.length > 1 ? (
				<SamePassword
					id="keys-download-same"
					same={same}
					onSame={setSame}
					password={samePassword}
					onPassword={setSamePassword}
				/>
			) : null}
			{error ? <InlineResult tone="critical">{error}</InlineResult> : null}
			{done ? <InlineResult tone={done.tone}>{done.text}</InlineResult> : null}
		</DvSheet>
	);
}
