"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CloudUpload,
	FileUp,
	LockOpen,
	RefreshCw,
	RotateCcw,
	ShieldCheck,
} from "lucide-react";
import { type FormEvent, useMemo, useRef, useState } from "react";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import {
	CheckField,
	Field,
	SecretInput,
	utf8Bytes,
} from "../primitives/form-fields";
import { GateInline } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { Meter } from "../primitives/meter";
import { StateView } from "../primitives/state-view";
import { type CheckSummary, checkSummary } from "./backup-check";
import type { KeyFlowsApi } from "./key-flows";
import { PASSWORD_MAX_BYTES, PASSWORD_MIN_BYTES } from "./key-operations";
import {
	KvHint,
	Mono,
	RowStatus,
	type RowStatusTone,
	SamePassword,
	SheetList,
	SheetListItem,
	keyKindLabel,
	useNames,
} from "./key-parts";
import { rowScope, useKeyResults } from "./key-store";
import { type KeyRow, localRevision, restoreCandidates } from "./keys-model";
import { BackupsStamp } from "./keys-table";
import { useFlowGuard, useKeyActions } from "./use-key-actions";
import type { KeysRead } from "./use-keys-model";

export const CHECK_RESULT = "check";
export const RESTORE_RESULT = "restore";

function tooShort(password: string): boolean {
	return utf8Bytes(password) < PASSWORD_MIN_BYTES;
}

/** "Check backups": reads the account's versions again and says what differs. Nothing is opened or sent. */
function useCheckBackups(read: KeysRead) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const names = useNames();
	const actions = useKeyActions();
	const results = useKeyResults();
	const guard = useFlowGuard();
	const latest = useRef(read);
	latest.current = read;
	const [busy, setBusy] = useState(false);

	const failed = (at: string): CheckSummary => {
		const from = latest.current.backups.checkedAt;
		return {
			tone: "warning",
			text:
				from === undefined
					? t(
							"keys.check.failedNoData",
							"Couldn't reach your account at {{at}}. Try again in a moment.",
							{ at },
						)
					: t(
							"keys.check.failed",
							"Couldn't reach your account at {{at}}. The versions shown are from {{from}}.",
							{ at, from: time.at(from) },
						),
		};
	};

	const run = async () => {
		if (busy) return;
		const flow = guard();
		setBusy(true);
		const { ok } = await read.backups.check();
		if (!flow.alive()) return;
		setBusy(false);
		const at = actions.timeNow();
		const summary = ok
			? checkSummary(t, names, latest.current.model, at)
			: failed(at);
		results.put(CHECK_RESULT, summary.tone, summary.text);
	};
	return { busy, run };
}

/** SPEC §5.9 "Your account": slots (BG25), what a backup holds, and "Check backups". */
export function AccountBackupPanel({ read }: Readonly<{ read: KeysRead }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const results = useKeyResults();
	const { busy, run: check } = useCheckBackups(read);
	const { model, backups } = read;
	const held = model.rows.filter((row) => row.hubRevision).length;
	const result = results.of(CHECK_RESULT);

	return (
		<Block
			icon={CloudUpload}
			title={t("keys.account.title", "Your account")}
			stamp={<BackupsStamp read={read} />}
		>
			<KeyValueList>
				<KvRow label={t("keys.account.backups", "Account backups")}>
					{backups.slots ? (
						<span className="inline-flex flex-wrap items-center gap-x-2.5 gap-y-1">
							<span>
								<Trans
									t={t}
									i18nKey="keys.account.slots"
									defaults="<1>{{used, number}}</1> of {{max, number}} slots"
									values={backups.slots}
									components={{
										1: <b className="font-semibold tabular-nums" />,
									}}
								/>
							</span>
							<Meter
								className="w-24"
								segments={[
									{
										value: Math.max(
											1.5,
											(backups.slots.used / Math.max(1, backups.slots.max)) *
												100,
										),
										tone:
											backups.slots.used >= backups.slots.max * (240 / 256)
												? "warning"
												: "good",
									},
								]}
								label={t(
									"keys.account.slotsAria",
									"{{used, number}} of {{max, number}} account backup slots used",
									backups.slots,
								)}
							/>
						</span>
					) : backups.interim ? (
						t(
							"keys.account.interim",
							"{{count, number}} for the devices in your list",
							{ count: held },
						)
					) : (
						<span className="text-muted-foreground">
							{t("keys.account.loading", "Checking…")}
						</span>
					)}
					<KvHint>
						{backups.interim
							? t(
									"keys.account.interimHint",
									"This hub reports backups one device at a time: versions only, without the save date or how many slots are in use.",
								)
							: t(
									"keys.account.slotsHint",
									"One slot per active device and per setup package that can still be used. Backups for revoked devices and lapsed setups hold none; the hub removes them when the next backup is saved.",
								)}
					</KvHint>
				</KvRow>
				<KvRow label={t("keys.account.holds", "What one holds")}>
					{t(
						"keys.account.holdsValue",
						"A device's keys, encrypted with its device password",
					)}
					<KvHint>
						{t(
							"keys.account.holdsHint",
							"The hub stores it encrypted. You still need the device password to open it.",
						)}
					</KvHint>
				</KvRow>
				<KvRow label={t("keys.account.lastCheck", "Last check")}>
					{backups.checkedAt === undefined ? (
						<span className="text-muted-foreground">
							{t("keys.account.notChecked", "Not checked yet")}
						</span>
					) : (
						<span title={time.abs(backups.checkedAt)}>
							{t(
								"keys.account.lastCheckValue",
								"{{ago}} · compared versions only",
								{
									ago: time.ago(Math.min(backups.checkedAt, time.nowS), "long"),
								},
							)}
						</span>
					)}
					<KvHint>
						{t(
							"keys.account.lastCheckHint",
							"Checking compares version numbers. Opening a backup needs its device password.",
						)}
					</KvHint>
				</KvRow>
			</KeyValueList>
			<div className="flex flex-wrap items-center gap-2">
				<DvButton
					size="sm"
					icon={RefreshCw}
					busy={busy}
					onClick={() => void check()}
				>
					{t("keys.account.check", "Check backups")}
				</DvButton>
				<span className="text-xs text-muted-foreground">
					{t(
						"keys.account.sharedHint",
						"A shared-access backup can be saved once the owner approves your request.",
					)}
				</span>
			</div>
			{result ? (
				<InlineResult
					tone={result.tone}
					onDismiss={() => results.dismiss(CHECK_RESULT)}
				>
					{result.text}
				</InlineResult>
			) : null}
		</Block>
	);
}

export type BackupMode = "save" | "update" | "check";

interface BackupSheetProps {
	row: KeyRow;
	mode: BackupMode;
	onClose(): void;
}

/** The row result once a backup was saved, updated or checked. */
function backupResultText(
	t: DevicesT,
	mode: BackupMode,
	version: number,
	at: string,
): string {
	const texts: Record<BackupMode, string> = {
		check: t(
			"devices:keys.result.checked",
			"Checked at {{at}}: your account's backup v{{version}} opens with that password. This computer now tracks that version.",
			{ at, version },
		),
		update: t(
			"devices:keys.result.updated",
			"Account backup updated as version {{version}} at {{at}}. It opens with the password you just typed.",
			{ at, version },
		),
		save: t(
			"devices:keys.result.backedUp",
			"Backed up to your account as version {{version}} at {{at}}.",
			{ at, version },
		),
	};
	return texts[mode];
}

/**
 * Back up, update or check one device's account backup. Sealing and opening
 * need the device password itself, so it is typed here, used once and cleared
 * as the request starts. Another device or mode starts a fresh form, so a
 * late answer of the previous one reaches nobody.
 */
export function AccountBackupSheet(props: Readonly<BackupSheetProps>) {
	return (
		<BackupSheetForm key={`${props.row.deviceId}:${props.mode}`} {...props} />
	);
}

function BackupSheetForm({ row, mode, onClose }: Readonly<BackupSheetProps>) {
	const { t } = useTranslation("devices");
	const actions = useKeyActions();
	const results = useKeyResults();
	const guard = useFlowGuard();
	const working = useRef(false);
	const [password, setPassword] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const device = row.name;
	const target = useMemo(
		() => ({ deviceId: row.deviceId, name: device }),
		[row.deviceId, device],
	);
	const hub = row.hubRevision ?? 0;
	const local = localRevision(row);

	const submit = async (event: FormEvent) => {
		event.preventDefault();
		if (working.current || tooShort(password)) return;
		const secret = password;
		const flow = guard();
		working.current = true;
		setPassword("");
		setError(undefined);
		setBusy(true);
		const send = mode === "check" ? actions.verifyBackup : actions.saveBackup;
		const outcome = await send(target, secret, flow.signal);
		if (!flow.alive()) return;
		working.current = false;
		setBusy(false);
		if (!outcome.ok) {
			setError(outcome.text || undefined);
			return;
		}
		results.put(
			rowScope(row.deviceId),
			"good",
			backupResultText(t, mode, outcome.value, actions.timeNow()),
		);
		onClose();
	};

	const title = {
		save: t("keys.backupSheet.saveTitle", "Back up the keys for {{device}}", {
			device,
		}),
		update: t(
			"keys.backupSheet.updateTitle",
			"Update the account backup for {{device}}",
			{ device },
		),
		check: t(
			"keys.backupSheet.checkTitle",
			"Check the account backup for {{device}}",
			{ device },
		),
	}[mode];
	const intro = {
		save: t(
			"keys.backupSheet.saveIntro",
			"The keys on this computer are sealed with the device password, here, and stored on your account. The hub can't open them, and nothing is sent to the device.",
		),
		update: t(
			"keys.backupSheet.updateIntro",
			"This computer's current keys are sealed again with the device password and replace version {{version}} on your account. Older backup files keep their own password.",
			{ version: hub },
		),
		check: t(
			"keys.backupSheet.checkIntro",
			"Checking downloads your account's backup and opens it with the device password, on this computer. If another computer saved a newer version, this computer adopts its version number. The keys and the password here stay as they are.",
		),
	}[mode];
	const formId = `keys-backup-${row.deviceId}`;
	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
			icon={mode === "check" ? ShieldCheck : CloudUpload}
			title={title}
			sub={t(
				"keys.backupSheet.versions",
				"This computer v{{local}} · your account v{{hub}}",
				{ local, hub },
			)}
			footNote={t(
				"keys.backupSheet.footNote",
				"The password never leaves this computer.",
			)}
			foot={
				<>
					<DvButton onClick={onClose}>
						{t("keys.sheet.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						type="submit"
						form={formId}
						icon={mode === "check" ? ShieldCheck : CloudUpload}
						busy={busy}
						aria-disabled={tooShort(password) || undefined}
					>
						{
							{
								save: t("keys.backup.backUp", "Back up to account"),
								update: t("keys.backup.update", "Update account backup"),
								check: t("keys.backupSheet.check", "Check backup"),
							}[mode]
						}
					</DvButton>
				</>
			}
		>
			<form
				id={formId}
				className="flex flex-col gap-3.5"
				onSubmit={(event) => void submit(event)}
			>
				<p className="text-sm">{intro}</p>
				<Field
					id={`${formId}-password`}
					label={t(
						"keys.field.devicePassword",
						"Device password for {{device}}",
						{ device },
					)}
					error={error}
				>
					<SecretInput
						value={password}
						onValueChange={setPassword}
						minBytes={PASSWORD_MIN_BYTES}
						maxBytes={PASSWORD_MAX_BYTES}
						disabled={busy}
						autoFocus
					/>
				</Field>
			</form>
		</DvSheet>
	);
}

type RestorePhase = "form" | "progress" | "done";

interface RestoreRowState {
	tone: RowStatusTone;
	text: string;
}

interface RestoreSheetProps {
	read: KeysRead;
	preselect?: string;
	flows: KeyFlowsApi;
	onClose(): void;
}

/** Until the device list answered there is nothing to offer, and that is not "every device has keys" (R6). */
function RestoreWaiting({
	read,
	onClose,
}: Readonly<Pick<RestoreSheetProps, "read" | "onClose">>) {
	const { t } = useTranslation("devices");
	const failed = read.devices.freshness.age === "error";
	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
			icon={RotateCcw}
			title={t("keys.restore.title", "Restore keys from your account")}
			foot={
				<DvButton onClick={onClose}>{t("keys.sheet.close", "Close")}</DvButton>
			}
		>
			<StateView
				kind={failed ? "error" : "loading"}
				title={
					failed
						? t(
								"keys.table.listError",
								"Couldn't load your device list from the hub.",
							)
						: t("keys.table.loading", "Loading your devices…")
				}
				text={t(
					"keys.restore.listPending",
					"Restoring offers the devices in your list that have no keys here, so it needs that list first.",
				)}
			/>
		</DvSheet>
	);
}

/** SPEC §5.9 "Restore keys…": pick devices without keys here, one backup password each. */
export function RestoreKeysSheet(props: Readonly<RestoreSheetProps>) {
	return props.read.devices.loaded ? (
		<RestoreKeysForm {...props} />
	) : (
		<RestoreWaiting read={props.read} onClose={props.onClose} />
	);
}

function RestoreKeysForm({
	read,
	preselect,
	flows,
	onClose,
}: Readonly<RestoreSheetProps>) {
	const { t } = useTranslation("devices");
	const names = useNames();
	const actions = useKeyActions();
	const results = useKeyResults();
	const guard = useFlowGuard();
	const working = useRef(false);
	const [candidates] = useState(() => restoreCandidates(read.model));
	const offered = (row: KeyRow) => row.hubRevision !== 0;
	const [selected, setSelected] = useState<Record<string, boolean>>(() =>
		Object.fromEntries(
			candidates.map((row) => [
				row.deviceId,
				offered(row) && (!preselect || preselect === row.deviceId),
			]),
		),
	);
	const [passwords, setPasswords] = useState<Record<string, string>>({});
	const [same, setSame] = useState(false);
	const [samePassword, setSamePassword] = useState("");
	const [phase, setPhase] = useState<RestorePhase>("form");
	const [states, setStates] = useState<Record<string, RestoreRowState>>({});
	const [error, setError] = useState<string>();
	const [summary, setSummary] = useState<{
		tone: "good" | "warning";
		text: string;
		restored: number;
	}>();

	const chosen = candidates.filter(
		(row) => selected[row.deviceId] && offered(row),
	);
	const passwordOf = (row: KeyRow) =>
		same ? samePassword : (passwords[row.deviceId] ?? "");

	const clearSecrets = () => {
		setPasswords({});
		setSamePassword("");
	};

	const run = async () => {
		if (working.current || phase !== "form" || !chosen.length) return;
		const missing = chosen.filter((row) => !passwordOf(row));
		const short = chosen.filter((row) => tooShort(passwordOf(row)));
		if (missing.length || short.length) {
			setError(
				missing.length
					? t(
							"keys.restore.missing",
							"Enter the device password for {{devices}}.",
							{ devices: names(missing.map((row) => row.name)) },
						)
					: t(
							"keys.restore.short",
							"Device passwords are at least 12 bytes. Check what you typed for {{devices}}.",
							{ devices: names(short.map((row) => row.name)) },
						),
			);
			return;
		}
		const flow = guard();
		const jobs = chosen.map((row) => ({ row, secret: passwordOf(row) }));
		working.current = true;
		clearSecrets();
		setError(undefined);
		setPhase("progress");
		const ok: string[] = [];
		const failed: string[] = [];
		for (const { row, secret } of jobs) {
			setStates((current) => ({
				...current,
				[row.deviceId]: {
					tone: "info",
					text: t(
						"keys.restore.opening",
						"Downloading the backup and opening it…",
					),
				},
			}));
			const outcome = await actions.restore(
				{ deviceId: row.deviceId, name: row.name },
				secret,
				flow.signal,
			);
			if (!flow.alive()) return;
			if (outcome.ok) {
				ok.push(row.name);
				results.put(
					rowScope(row.deviceId),
					"good",
					t(
						"keys.result.restored",
						"Restored from your account backup (v{{version}}) at {{at}}. Unlock with the device password to use the keys.",
						{ version: outcome.value, at: actions.timeNow() },
					),
				);
			} else failed.push(row.name);
			setStates((current) => ({
				...current,
				[row.deviceId]: outcome.ok
					? {
							tone: "good",
							text: t(
								"keys.restore.rowDone",
								"Restored v{{version}} · locked",
								{
									version: outcome.value,
								},
							),
						}
					: { tone: "critical", text: outcome.text },
			}));
		}
		working.current = false;
		const at = actions.timeNow();
		const done = ok.length
			? t(
					"keys.restore.summaryOk",
					"Restored keys for {{devices}} at {{at}}. They're locked; unlock with each device's password.",
					{ devices: names(ok), at },
				)
			: t("keys.restore.summaryNone", "Nothing was restored at {{at}}.", {
					at,
				});
		const text = failed.length
			? `${done} ${t(
					"keys.restore.summaryFailed",
					"{{devices}}: not restored, see above.",
					{ devices: names(failed) },
				)}`
			: done;
		const tone = failed.length ? "warning" : "good";
		setSummary({ tone, text, restored: ok.length });
		results.put(RESTORE_RESULT, tone, text);
		setPhase("done");
	};

	const again = () => {
		clearSecrets();
		setStates({});
		setError(undefined);
		setSummary(undefined);
		setPhase("form");
	};

	const close = () => {
		clearSecrets();
		onClose();
	};

	const none = read.model.rows.filter(
		(row) => row.relationship === "cloud_approval",
	);
	const empty = candidates.length === 0;
	const foot = empty ? (
		<>
			<DvButton
				icon={FileUp}
				onClick={() => {
					close();
					flows.open({ kind: "import" });
				}}
			>
				{t("keys.header.import", "Import backup files…")}
			</DvButton>
			<DvButton onClick={close}>{t("keys.sheet.close", "Close")}</DvButton>
		</>
	) : phase === "form" ? (
		<>
			<DvButton onClick={close}>{t("keys.sheet.cancel", "Cancel")}</DvButton>
			<DvButton
				variant="primary"
				icon={RotateCcw}
				aria-disabled={!chosen.length || undefined}
				onClick={() => void run()}
			>
				{t("keys.restore.go", {
					count: chosen.length,
					defaultValue_one: "Restore {{count, number}} device",
					defaultValue_other: "Restore {{count, number}} devices",
				})}
			</DvButton>
		</>
	) : phase === "progress" ? null : summary?.restored ? (
		<>
			<DvButton
				icon={LockOpen}
				onClick={() => {
					close();
					flows.unlockSeveral();
				}}
			>
				{t("keys.unlockSeveral", "Unlock several…")}
			</DvButton>
			<DvButton variant="primary" onClick={close}>
				{t("keys.sheet.done", "Done")}
			</DvButton>
		</>
	) : (
		<>
			<DvButton icon={RotateCcw} onClick={again}>
				{t("keys.restore.again", "Try other passwords")}
			</DvButton>
			<DvButton variant="primary" onClick={close}>
				{t("keys.sheet.done", "Done")}
			</DvButton>
		</>
	);
	const footNote = empty
		? undefined
		: phase === "form"
			? t("keys.restore.noteForm", "Restored keys start locked.")
			: phase === "progress"
				? t(
						"keys.restore.noteProgress",
						"Restoring. Each device takes a few seconds.",
					)
				: summary?.restored
					? t(
							"keys.restore.noteDone",
							"Unlock with the same passwords to use them.",
						)
					: t("keys.restore.noteNone", "Nothing changed on this computer.");

	return (
		<DvSheet
			open
			wide
			closeOnOutside={phase !== "progress"}
			onOpenChange={(open) => {
				if (!open && phase !== "progress") close();
			}}
			icon={RotateCcw}
			title={t("keys.restore.title", "Restore keys from your account")}
			sub={t(
				"keys.restore.subtitle",
				"Pick the devices, then enter each device's password.",
			)}
			foot={foot}
			footNote={footNote}
		>
			{empty ? (
				<StateView
					kind="empty"
					icon={CircleCheck}
					title={t(
						"keys.restore.empty",
						"Every device you manage already has keys here.",
					)}
					text={
						<>
							{none.length
								? `${t("keys.restore.emptyConsent", {
										count: none.length,
										devices: names(none.map((row) => row.name)),
										defaultValue_one:
											"{{devices}} isn't offered: you only approve its cloud access, so there are no keys to restore.",
										defaultValue_other:
											"{{devices}} aren't offered: you only approve their cloud access, so there are no keys to restore.",
									})} `
								: null}
							{t(
								"keys.restore.emptyImport",
								"To bring keys from another computer, import its backup files.",
							)}
						</>
					}
				/>
			) : (
				<>
					<p className="text-sm">
						{t(
							"keys.restore.intro",
							"Restoring downloads each device's account backup and opens it with that device's password, on this computer. Nothing is sent to the device, and the hub never sees the password.",
						)}
					</p>
					<SheetList>
						{candidates.map((row) => {
							const state = states[row.deviceId];
							const id = `keys-restore-${row.deviceId}`;
							const canOffer = offered(row);
							return (
								<SheetListItem
									key={row.deviceId}
									data-restore-row={row.deviceId}
									head={
										<CheckField
											id={id}
											checked={Boolean(selected[row.deviceId]) && canOffer}
											disabled={!canOffer || phase !== "form"}
											onCheckedChange={(checked) =>
												setSelected((current) => ({
													...current,
													[row.deviceId]: checked,
												}))
											}
										>
											<Mono>{row.name}</Mono>
										</CheckField>
									}
									status={
										state ? (
											<RowStatus tone={state.tone}>{state.text}</RowStatus>
										) : (
											<RowStatus tone="muted">
												{!canOffer
													? t("keys.restore.rowNone", "no account backup")
													: row.hubRevision === undefined
														? t(
																"keys.restore.rowUnchecked",
																"{{kind}} · account backup not checked yet",
																{
																	kind: keyKindLabel(
																		t,
																		row.relationship === "owner"
																			? "owner"
																			: "shared",
																	),
																},
															)
														: t(
																"keys.restore.rowBackup",
																"{{kind}} · account backup v{{version}}",
																{
																	kind: keyKindLabel(
																		t,
																		row.relationship === "owner"
																			? "owner"
																			: "shared",
																	),
																	version: row.hubRevision,
																},
															)}
											</RowStatus>
										)
									}
								>
									{!canOffer ? (
										<GateInline kind="nokeys" className="max-w-none">
											{t(
												"keys.restore.rowNoneWhy",
												"No account backup. Import its backup file, or use the computer that set it up.",
											)}
										</GateInline>
									) : phase === "form" && !same && selected[row.deviceId] ? (
										<SecretInput
											value={passwords[row.deviceId] ?? ""}
											onValueChange={(value) =>
												setPasswords((current) => ({
													...current,
													[row.deviceId]: value,
												}))
											}
											aria-label={t(
												"keys.field.devicePassword",
												"Device password for {{device}}",
												{ device: row.name },
											)}
											placeholder={t(
												"keys.field.devicePassword",
												"Device password for {{device}}",
												{ device: row.name },
											)}
										/>
									) : null}
								</SheetListItem>
							);
						})}
					</SheetList>
					{phase === "form" && chosen.length > 1 ? (
						<SamePassword
							id="keys-restore-same"
							same={same}
							onSame={setSame}
							password={samePassword}
							onPassword={setSamePassword}
							hint={t(
								"keys.restore.sameHint",
								"Devices where it doesn't open the backup stay without keys; you can try them again.",
							)}
						/>
					) : null}
					{error ? <InlineResult tone="critical">{error}</InlineResult> : null}
					{phase === "done" && summary ? (
						<InlineResult tone={summary.tone}>{summary.text}</InlineResult>
					) : null}
				</>
			)}
		</DvSheet>
	);
}
